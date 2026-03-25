use std::sync::{Arc, atomic::Ordering};

use bitturbulence_transport::PeerConnection;

use super::context::TorrentCtx;
use super::drainer::run_peer_downloader;
use super::filler::run_peer_filler;

/// Punto de entrada por peer. Despacha a drainer (outbound) o filler (inbound).
pub async fn run_peer(
    conn:     PeerConnection,
    ctx:      Arc<TorrentCtx>,
    peer_id:  [u8; 32],
    outbound: bool,
) {
    let addr = conn.remote_addr();
    ctx.peer_count.fetch_add(1, Ordering::Relaxed);

    let result = if outbound {
        run_peer_downloader(&conn, &ctx, &peer_id).await
    } else {
        run_peer_filler(&conn, &ctx, &peer_id).await
    };

    if let Err(e) = result {
        tracing::debug!(%addr, "peer: {e}");
    }

    ctx.peer_count.fetch_sub(1, Ordering::Relaxed);
}
