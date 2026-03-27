use std::sync::{atomic::Ordering, Arc};

use tracing::Instrument;

use bitturbulence_transport::PeerConnection;

use super::context::FlowCtx;
use super::drainer::run_peer_downloader;
use super::filler::run_peer_filler;

/// Punto de entrada por peer. Despacha a drainer (outbound) o filler (inbound).
pub async fn run_peer(conn: PeerConnection, ctx: Arc<FlowCtx>, peer_id: [u8; 32], outbound: bool) {
    let addr = conn.remote_addr();
    let span = tracing::info_span!(
        "peer",
        %addr,
        peer_id = %hex::encode(&peer_id[..4]),
        info_hash = %hex::encode(&ctx.meta.info_hash[..4]),
    );

    async move {
        ctx.peer_count.fetch_add(1, Ordering::Relaxed);

        let result = if outbound {
            run_peer_downloader(&conn, &ctx, &peer_id).await
        } else {
            run_peer_filler(&conn, &ctx, &peer_id).await
        };

        if let Err(e) = result {
            tracing::debug!("peer: {e}");
        }

        ctx.peer_count.fetch_sub(1, Ordering::Relaxed);
    }
    .instrument(span)
    .await
}
