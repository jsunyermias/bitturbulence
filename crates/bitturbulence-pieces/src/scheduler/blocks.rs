use super::BlockScheduler;
use super::types::BlockState;

impl BlockScheduler {
    /// Marca el bloque `(pi, bi)` como completado y almacena su hash SHA-256.
    ///
    /// Devuelve `true` si es el primer stream en completar todos los bloques
    /// de la pieza — señal para iniciar la verificación Merkle.
    /// Solo devuelve `true` una vez por pieza.
    pub fn mark_block_done(&mut self, pi: u32, bi: u32, block_hash: [u8; 32]) -> bool {
        let pi = pi as usize;
        let bi = bi as usize;
        if pi >= self.num_pieces { return false; }
        if self.piece_done[pi] || self.piece_verifying[pi] { return false; }
        if bi >= self.block_state[pi].len() { return false; }

        self.block_state[pi][bi]  = BlockState::Done;
        self.block_hashes[pi][bi] = Some(block_hash);

        let all_done = self.block_state[pi].iter().all(|s| *s == BlockState::Done);
        if all_done {
            self.piece_verifying[pi] = true;
        }
        all_done
    }

    /// Devuelve los hashes SHA-256 de todos los bloques de la pieza `pi`.
    ///
    /// Solo válido cuando todos los bloques están `Done`. Con estos hashes
    /// se calcula la raíz Merkle sin releer el disco.
    pub fn piece_block_hashes(&self, pi: u32) -> Option<Vec<[u8; 32]>> {
        let pi = pi as usize;
        if pi >= self.num_pieces { return None; }
        self.block_hashes[pi].iter().copied().collect()
    }

    /// Decrementa el contador in-flight de un bloque (fallo o cancelación).
    ///
    /// Si el bloque ya está `Done` (otro stream llegó primero), no hace nada.
    pub fn mark_block_failed(&mut self, pi: u32, bi: u32) {
        let pi = pi as usize;
        let bi = bi as usize;
        if pi >= self.num_pieces || bi >= self.block_state[pi].len() { return; }
        if let BlockState::InFlight(n) = self.block_state[pi][bi] {
            self.block_state[pi][bi] = if n <= 1 {
                BlockState::Pending
            } else {
                BlockState::InFlight(n - 1)
            };
        }
    }

    /// Marca la pieza como verificada y descargada completamente.
    pub fn mark_piece_verified(&mut self, pi: u32) {
        let pi = pi as usize;
        if pi < self.num_pieces {
            self.piece_done[pi]      = true;
            self.piece_verifying[pi] = false;
        }
    }

    /// La verificación Merkle de la pieza ha fallado: resetea todos los
    /// bloques a `Pending` y borra los hashes para reintentar.
    pub fn mark_piece_hash_failed(&mut self, pi: u32) {
        let pi = pi as usize;
        if pi < self.num_pieces {
            self.piece_verifying[pi] = false;
            for b in &mut self.block_state[pi] { *b = BlockState::Pending; }
            for h in &mut self.block_hashes[pi] { *h = None; }
        }
    }

    // ── Consultas ─────────────────────────────────────────────────────────────

    /// Disponibilidad mínima entre todas las piezas que aún tienen trabajo pendiente.
    ///
    /// Devuelve `u32::MAX` si no hay piezas pendientes (archivo completo o
    /// todas verificando), lo que descarta el archivo de los passes de rareza.
    pub fn min_availability(&self) -> u32 {
        (0..self.num_pieces)
            .filter(|&pi| {
                !self.piece_done[pi]
                    && !self.piece_verifying[pi]
                    && self.block_state[pi].iter().any(|s| *s != BlockState::Done)
            })
            .map(|pi| self.availability[pi])
            .min()
            .unwrap_or(u32::MAX)
    }

    /// Devuelve `true` si todas las piezas están verificadas.
    pub fn is_complete(&self) -> bool {
        self.piece_done.iter().all(|&d| d)
    }

    /// Número de piezas verificadas.
    pub fn num_complete_pieces(&self) -> usize {
        self.piece_done.iter().filter(|&&d| d).count()
    }

    /// Número total de piezas.
    pub fn num_pieces(&self) -> usize {
        self.num_pieces
    }

    /// Fracción de piezas completadas (0.0–1.0).
    pub fn progress(&self) -> f32 {
        if self.num_pieces == 0 { return 1.0; }
        self.num_complete_pieces() as f32 / self.num_pieces as f32
    }
}
