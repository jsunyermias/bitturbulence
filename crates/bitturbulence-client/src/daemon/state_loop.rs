use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::time::interval;
use tracing::{info, warn};

use super::context::FlowCtx;
use super::STATE_SAVE_INTERVAL;
use crate::state::{ClientState, DownloadState};

pub async fn state_save_loop(
    state_path:  std::path::PathBuf,
    flow_ids:    Vec<(String, Arc<FlowCtx>)>,
) {
    let mut timer = interval(STATE_SAVE_INTERVAL);
    timer.tick().await;

    loop {
        timer.tick().await;

        let mut state = match ClientState::load(&state_path) {
            Ok(s)  => s,
            Err(e) => { warn!("load state: {e}"); continue; }
        };

        for (id, ctx) in &flow_ids {
            if let Some(entry) = state.get_mut(id) {
                entry.downloaded = ctx.downloaded.load(Ordering::Relaxed);
                entry.peers      = ctx.peer_count.load(Ordering::Relaxed);
                if ctx.is_complete().await && entry.state == DownloadState::Downloading {
                    entry.state = DownloadState::Seeding;
                    info!("[{id}] {} — completed!", entry.name);
                }
            }
        }

        if let Err(e) = state.save(&state_path) {
            warn!("save state: {e}");
        }
    }
}
