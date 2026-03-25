use bitturbulence_protocol::BLOCK_SIZE;

use super::BlockScheduler;
use super::types::{BlockState, BlockTask, block_length, piece_len, TYPICAL_STREAMS_PER_BLOCK, MAX_STREAMS_PER_BLOCK};

impl BlockScheduler {
    /// Selecciona el siguiente bloque a descargar para un stream QUIC.
    ///
    /// `peer_pieces[pi]` indica si el peer tiene la pieza `pi`.
    ///
    /// Prioridad aplicada:
    /// 1. Bloque `Pending` en pieza rarest-first que el peer tiene.
    /// 2. Bloque `InFlight(n < TYPICAL)` — añade redundancia hasta el objetivo.
    /// 3. Bloque `InFlight(n < MAX)` — endgame.
    ///
    /// Marca el bloque seleccionado como `InFlight(n+1)` antes de devolver.
    pub fn schedule(&mut self, peer_pieces: &[bool]) -> Option<BlockTask> {
        if let Some(t) = self.find_block(peer_pieces, |s| *s == BlockState::Pending) {
            return Some(t);
        }
        if let Some(t) = self.find_block(peer_pieces, |s| {
            matches!(s, BlockState::InFlight(n) if *n < TYPICAL_STREAMS_PER_BLOCK)
        }) {
            return Some(t);
        }
        self.find_block(peer_pieces, |s| {
            matches!(s, BlockState::InFlight(n) if *n < MAX_STREAMS_PER_BLOCK)
        })
    }

    /// Como [`schedule`], pero solo devuelve bloques `Pending`.
    ///
    /// Usado para decidir si merece la pena abrir un nuevo stream: solo
    /// se abre si hay trabajo genuinamente nuevo, nunca solo para redundancia.
    pub fn schedule_pending(&mut self, peer_pieces: &[bool]) -> Option<BlockTask> {
        self.find_block(peer_pieces, |s| *s == BlockState::Pending)
    }

    /// Busca el primer bloque que satisface `predicate` en la pieza más rara
    /// que el peer tiene, marca el bloque como `InFlight(n+1)` y devuelve la tarea.
    fn find_block<F>(&mut self, peer_pieces: &[bool], predicate: F) -> Option<BlockTask>
    where
        F: Fn(&BlockState) -> bool,
    {
        // Selección rarest-first: pieza candidata con menor availability.
        let mut best: Option<(usize, u32)> = None;

        for pi in 0..self.num_pieces {
            if self.piece_done[pi] || self.piece_verifying[pi] { continue; }
            if !peer_pieces.get(pi).copied().unwrap_or(false) { continue; }
            if !self.block_state[pi].iter().any(|s| predicate(s)) { continue; }

            let avail = self.availability[pi];
            match best {
                None => best = Some((pi, avail)),
                Some((_, ba)) if avail < ba => best = Some((pi, avail)),
                _ => {}
            }
        }

        let (pi, _) = best?;

        for bi in 0..self.block_state[pi].len() {
            if predicate(&self.block_state[pi][bi]) {
                let n = match self.block_state[pi][bi] {
                    BlockState::Pending     => 1,
                    BlockState::InFlight(n) => n + 1,
                    BlockState::Done        => continue,
                };
                self.block_state[pi][bi] = BlockState::InFlight(n);

                let begin  = bi as u32 * BLOCK_SIZE;
                let length = block_length(pi, bi, self.num_pieces, self.piece_length, self.last_piece_len);
                return Some(BlockTask { fi: self.fi, pi: pi as u32, bi: bi as u32, begin, length });
            }
        }

        None
    }
}
