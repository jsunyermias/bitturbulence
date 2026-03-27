use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::time::interval;
use tracing::warn;

use super::context::FlowCtx;
use super::STATE_SAVE_INTERVAL;
use crate::state::{ClientState, DownloadState};

pub async fn state_save_loop(
    state_path:  std::path::PathBuf,
    flow_ids:    Vec<(String, Arc<FlowCtx>)>,
) {
    // Por cada flow que aún no está completo, lanzar un watcher dedicado que
    // actualiza el estado a Seeding inmediatamente al recibir la señal.
    for (id, ctx) in &flow_ids {
        let mut complete_rx = ctx.complete_tx.subscribe();
        if !*complete_rx.borrow() {
            let id         = id.clone();
            let ctx        = ctx.clone();
            let state_path = state_path.clone();
            tokio::spawn(async move {
                // Esperar a que la descarga complete.
                let _ = complete_rx.changed().await;
                if !*complete_rx.borrow() { return; }

                // Guardar estado inmediatamente como Seeding.
                let mut state = match ClientState::load(&state_path) {
                    Ok(s)  => s,
                    Err(e) => { warn!("load state on completion: {e}"); return; }
                };
                if let Some(entry) = state.get_mut(&id) {
                    if entry.state == DownloadState::Downloading {
                        entry.state      = DownloadState::Seeding;
                        entry.downloaded = ctx.downloaded.load(Ordering::Relaxed);
                        if let Err(e) = state.save(&state_path) {
                            warn!("save state on completion: {e}");
                        }
                    }
                }
            });
        }
    }

    // Bucle de guardado periódico (progreso, peer count, etc.).
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
                // El watch ya habrá actualizado el estado a Seeding si completó;
                // aquí solo sincronizamos si el watch se perdió por algún motivo.
                if *ctx.complete_tx.borrow() && entry.state == DownloadState::Downloading {
                    entry.state = DownloadState::Seeding;
                }
            }
        }

        if let Err(e) = state.save(&state_path) {
            warn!("save state: {e}");
        }
    }
}
