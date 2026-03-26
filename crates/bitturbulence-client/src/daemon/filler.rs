use std::sync::Arc;

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use tokio::time::interval;
use tracing::info;

use bitturbulence_protocol::Message;
use bitturbulence_transport::PeerConnection;

use super::context::FlowCtx;
use super::drainer::send_our_bitfields;
use super::stream::serve_data_stream;
use super::KEEPALIVE_INTERVAL;

// ── Bucle del filler (conexión entrante) ──────────────────────────────────────

pub async fn run_peer_filler(
    conn:     &PeerConnection,
    ctx:      &Arc<FlowCtx>,
    _peer_id: &[u8; 32],
) -> Result<()> {
    // Stream 1 ya fue gestionado por handle_inbound (Hello / HelloAck).
    // Aceptamos el stream 2 como canal de control.
    let (mut ctrl_w, mut ctrl_r) = conn.accept_bidi_stream().await?;

    // Anunciar nuestra disponibilidad al drainer.
    send_our_bitfields(ctx, &mut ctrl_w).await?;

    let mut ka_timer = interval(KEEPALIVE_INTERVAL);
    ka_timer.tick().await;

    loop {
        tokio::select! {
            msg_opt = ctrl_r.next() => {
                match msg_opt {
                    Some(Ok(Message::KeepAlive))          => {}
                    Some(Ok(Message::HavePiece { .. }))   => {}
                    Some(Ok(Message::HaveNone  { .. }))   => {}
                    Some(Ok(Message::HaveAll   { .. }))   => {}
                    Some(Ok(Message::HaveBitmap { .. }))  => {}
                    Some(Ok(Message::Bye { reason }))     => {
                        info!("peer bye: {reason}");
                        return Ok(());
                    }
                    Some(Ok(_))  => {}
                    Some(Err(e)) => return Err(e.into()),
                    None         => return Ok(()),
                }
            }

            // Aceptar nuevos data streams del drainer.
            stream = conn.accept_bidi_stream() => {
                match stream {
                    Ok((w, r)) => {
                        tokio::spawn(serve_data_stream(w, r, ctx.clone()));
                    }
                    Err(e) => {
                        tracing::debug!("accept data stream: {e}");
                        break;
                    }
                }
            }

            _ = ka_timer.tick() => {
                ctrl_w.send(Message::KeepAlive).await?;
            }
        }
    }

    Ok(())
}
