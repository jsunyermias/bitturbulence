use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::time::timeout;
use tracing::{debug, info, warn};

use bitturbulence_protocol::Message;
use bitturbulence_transport::{PeerConnection, QuicEndpoint};

use super::context::TorrentCtx;
use super::peer::run_peer;
use super::HELLO_TIMEOUT;

pub async fn handle_inbound(
    conn:     PeerConnection,
    torrents: Arc<HashMap<[u8; 32], Arc<TorrentCtx>>>,
    peer_id:  [u8; 32],
) -> Result<()> {
    // Stream 1: Hello / HelloAck
    let (mut hello_w, mut hello_r) = conn.accept_bidi_stream().await?;

    let msg = timeout(HELLO_TIMEOUT, hello_r.next()).await
        .map_err(|_| anyhow!("hello timeout"))?
        .ok_or_else(|| anyhow!("disconnected during hello"))??;

    let info_hash = match msg {
        Message::Hello { info_hash, .. } => info_hash,
        _ => return Err(anyhow!("expected Hello")),
    };

    match torrents.get(&info_hash) {
        Some(ctx) => {
            let ack = Message::HelloAck { peer_id, accepted: true, reason: None };
            timeout(HELLO_TIMEOUT, hello_w.send(ack)).await
                .map_err(|_| anyhow!("ack timeout"))??;
            drop(hello_w);
            drop(hello_r);
            run_peer(conn, ctx.clone(), peer_id, false).await;
            Ok(())
        }
        None => {
            let ack = Message::HelloAck {
                peer_id,
                accepted: false,
                reason:   Some("flow not found".into()),
            };
            let _ = timeout(HELLO_TIMEOUT, hello_w.send(ack)).await;
            Ok(())
        }
    }
}

pub async fn accept_loop(
    endpoint: Arc<QuicEndpoint>,
    torrents: Arc<HashMap<[u8; 32], Arc<TorrentCtx>>>,
    peer_id:  [u8; 32],
) {
    loop {
        match endpoint.accept().await {
            None => { info!("QUIC endpoint closed"); break; }
            Some(Err(e)) => { warn!("accept error: {e}"); }
            Some(Ok(conn)) => {
                let torrents = torrents.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_inbound(conn, torrents, peer_id).await {
                        debug!("inbound handshake: {e}");
                    }
                });
            }
        }
    }
}
