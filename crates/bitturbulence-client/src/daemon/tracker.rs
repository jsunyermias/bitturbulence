use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::time::interval;
use tracing::{info, warn};

use bitturbulence_tracker::{AnnounceEvent, AnnounceRequest, TrackerClient};
use bitturbulence_transport::QuicEndpoint;

use super::context::FlowCtx;
use super::peer::run_peer;
use super::ANNOUNCE_INTERVAL;

pub async fn tracker_loop(
    tracker_url:  String,
    ctx:          Arc<FlowCtx>,
    endpoint:     Arc<QuicEndpoint>,
    peer_id:      [u8; 32],
    listen_port:  u16,
    known_peers:  Arc<Mutex<HashSet<String>>>,
) {
    let client = match TrackerClient::new(&tracker_url, None) {
        Ok(c)  => c,
        Err(e) => { warn!("tracker client: {e}"); return; }
    };

    let listen_addr = format!("127.0.0.1:{listen_port}");
    let mut timer   = interval(ANNOUNCE_INTERVAL);
    let mut event   = AnnounceEvent::Started;

    loop {
        timer.tick().await;

        let downloaded = ctx.downloaded.load(std::sync::atomic::Ordering::Relaxed);
        let left       = ctx.meta.total_size().saturating_sub(downloaded);

        let req = AnnounceRequest::new(
            &ctx.meta.info_hash, &peer_id, &listen_addr,
            event, 0, downloaded, left,
        );

        match client.announce(req).await {
            Ok(resp) => {
                info!(tracker = %tracker_url, peers = resp.peers.len(), "announce ok");

                for peer_info in resp.peers {
                    let addr_str = peer_info.addr.clone();
                    let is_new   = known_peers.lock().await.insert(addr_str.clone());
                    if !is_new { continue; }

                    let ep   = endpoint.clone();
                    let ctx2 = ctx.clone();
                    tokio::spawn(async move {
                        match addr_str.parse::<std::net::SocketAddr>() {
                            Ok(addr) => match ep.connect(addr).await {
                                Ok(conn) => run_peer(conn, ctx2, peer_id, true).await,
                                Err(e)   => warn!("connect {addr}: {e}"),
                            },
                            Err(_) => warn!("invalid peer addr: {addr_str}"),
                        }
                    });
                }

                event = if ctx.is_complete().await {
                    AnnounceEvent::Completed
                } else {
                    AnnounceEvent::Update
                };
            }
            Err(e) => warn!("announce to {tracker_url}: {e}"),
        }
    }
}
