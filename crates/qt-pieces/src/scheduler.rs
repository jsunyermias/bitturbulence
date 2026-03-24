//! Scheduler de bloques con multiplexación QUIC.
//!
//! ## Modelo
//!
//! Un **bloque** (16 KiB) es la unidad atómica de Request/Piece en el wire.
//! Una **pieza** es un grupo de bloques y es la unidad de verificación SHA-256.
//!
//! Cada bloque puede estar siendo descargado por hasta [`MAX_STREAMS_PER_BLOCK`]
//! streams simultáneos. El scheduler prioriza:
//!
//! 1. Bloques `Pending` (nuevos) en piezas rarest-first.
//! 2. Bloques `InFlight(n < TYPICAL)` — añadir segundo stream para redundancia.
//! 3. Bloques `InFlight(n < MAX)` — solo cuando no hay nada mejor (endgame).
//!
//! Este orden hace que el número típico de streams por bloque sea 2 sin
//! desperdiciar streams en redundancia cuando hay piezas nuevas que descargar.

use qt_protocol::BLOCK_SIZE;

/// Streams típicos por bloque (objetivo bajo carga normal).
pub const TYPICAL_STREAMS_PER_BLOCK: u8 = 2;

/// Máximo de streams simultáneos por bloque.
pub const MAX_STREAMS_PER_BLOCK: u8 = 4;

// ── Estado de un bloque ───────────────────────────────────────────────────────

/// Estado de un bloque individual (16 KiB).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockState {
    /// No solicitado todavía.
    Pending,
    /// Siendo descargado por `n` streams simultáneos.
    InFlight(u8),
    /// Datos recibidos y escritos en disco.
    Done,
}

// ── Tarea de bloque ───────────────────────────────────────────────────────────

/// Tarea de descarga de un bloque individual.
#[derive(Debug, Clone)]
pub struct BlockTask {
    /// Índice del archivo en el torrent.
    pub fi: usize,
    /// Índice de la pieza.
    pub pi: u32,
    /// Índice del bloque dentro de la pieza (bi * BLOCK_SIZE = begin).
    pub bi: u32,
    /// Offset en bytes dentro de la pieza.
    pub begin: u32,
    /// Longitud del bloque (normalmente BLOCK_SIZE, el último puede ser menor).
    pub length: u32,
}

// ── BlockScheduler ────────────────────────────────────────────────────────────

/// Scheduler de descarga a nivel de bloque para un único archivo.
///
/// Gestiona el estado de cada bloque de cada pieza y expone métodos para
/// asignar trabajo a streams QUIC respetando los límites de multiplexación.
pub struct BlockScheduler {
    fi:           usize,
    piece_length: u32,
    last_piece_len: u32,
    num_pieces:   usize,

    /// Estado por bloque: `block_state[pi][bi]`.
    block_state: Vec<Vec<BlockState>>,

    /// Pieza marcada como totalmente descargada y con hash verificado.
    piece_done: Vec<bool>,

    /// Pieza en proceso de verificación (evita doble verificación).
    piece_verifying: Vec<bool>,

    /// Cuántos peers tienen cada pieza (rarest-first).
    availability: Vec<u32>,
}

impl BlockScheduler {
    /// Crea un scheduler para el archivo `fi`.
    ///
    /// `piece_length` debe ser múltiplo de [`BLOCK_SIZE`].
    /// `last_piece_len` es la longitud real de la última pieza.
    pub fn new(fi: usize, num_pieces: usize, piece_length: u32, last_piece_len: u32) -> Self {
        debug_assert!(
            piece_length % BLOCK_SIZE == 0 || num_pieces == 0,
            "piece_length={piece_length} no es múltiplo de BLOCK_SIZE={BLOCK_SIZE}"
        );

        let block_state = (0..num_pieces)
            .map(|pi| {
                let pl = if pi + 1 == num_pieces { last_piece_len } else { piece_length };
                let nb = pl.div_ceil(BLOCK_SIZE) as usize;
                vec![BlockState::Pending; nb]
            })
            .collect();

        Self {
            fi,
            piece_length,
            last_piece_len,
            num_pieces,
            block_state,
            piece_done:      vec![false; num_pieces],
            piece_verifying: vec![false; num_pieces],
            availability:    vec![0u32;  num_pieces],
        }
    }

    // ── Disponibilidad de peers ───────────────────────────────────────────────

    /// Registra el bitfield de un peer nuevo.
    pub fn add_peer_bitfield(&mut self, bitfield: &[bool]) {
        for (i, &has) in bitfield.iter().enumerate() {
            if i < self.num_pieces && has {
                self.availability[i] += 1;
            }
        }
    }

    /// El peer anuncia que tiene la pieza `pi`.
    pub fn add_peer_have(&mut self, pi: usize) {
        if pi < self.num_pieces {
            self.availability[pi] += 1;
        }
    }

    /// Un peer se desconecta — restamos su contribución.
    pub fn remove_peer_bitfield(&mut self, bitfield: &[bool]) {
        for (i, &had) in bitfield.iter().enumerate() {
            if i < self.num_pieces && had && self.availability[i] > 0 {
                self.availability[i] -= 1;
            }
        }
    }

    // ── Estado de bloques ─────────────────────────────────────────────────────

    /// Marca el bloque `(pi, bi)` como completado (datos escritos en disco).
    ///
    /// Devuelve `true` si es el primer stream en completar todos los bloques
    /// de la pieza (es decir, la pieza está lista para verificación SHA-256).
    /// Solo devuelve `true` una vez por pieza; llamadas posteriores con la
    /// pieza ya en verificación o completada devuelven `false`.
    pub fn mark_block_done(&mut self, pi: u32, bi: u32) -> bool {
        let pi = pi as usize;
        let bi = bi as usize;
        if pi >= self.num_pieces { return false; }
        if self.piece_done[pi] || self.piece_verifying[pi] { return false; }
        if bi >= self.block_state[pi].len() { return false; }

        self.block_state[pi][bi] = BlockState::Done;

        let all_done = self.block_state[pi].iter().all(|s| *s == BlockState::Done);
        if all_done {
            self.piece_verifying[pi] = true;
        }
        all_done
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

    /// La verificación SHA-256 de la pieza ha fallado: resetea todos los
    /// bloques a `Pending` para reintentarla desde cero.
    pub fn mark_piece_hash_failed(&mut self, pi: u32) {
        let pi = pi as usize;
        if pi < self.num_pieces {
            self.piece_verifying[pi] = false;
            for b in &mut self.block_state[pi] {
                *b = BlockState::Pending;
            }
        }
    }

    // ── Scheduling ────────────────────────────────────────────────────────────

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
        // Fase 1: bloques Pending (preferencia absoluta)
        if let Some(t) = self.find_block(peer_pieces, |s| *s == BlockState::Pending) {
            return Some(t);
        }
        // Fase 2: añadir segundo stream (típico)
        if let Some(t) = self.find_block(peer_pieces, |s| {
            matches!(s, BlockState::InFlight(n) if *n < TYPICAL_STREAMS_PER_BLOCK)
        }) {
            return Some(t);
        }
        // Fase 3: endgame — hasta MAX streams por bloque
        self.find_block(peer_pieces, |s| {
            matches!(s, BlockState::InFlight(n) if *n < MAX_STREAMS_PER_BLOCK)
        })
    }

    /// Como [`schedule`], pero solo devuelve bloques `Pending`.
    ///
    /// Usado para decidir si merece la pena **abrir un nuevo stream**: solo
    /// se abre si hay trabajo genuinamente nuevo, nunca solo para redundancia.
    pub fn schedule_pending(&mut self, peer_pieces: &[bool]) -> Option<BlockTask> {
        self.find_block(peer_pieces, |s| *s == BlockState::Pending)
    }

    // ── Consultas ─────────────────────────────────────────────────────────────

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

    // ── Helpers internos ──────────────────────────────────────────────────────

    fn piece_len(&self, pi: usize) -> u32 {
        if pi + 1 == self.num_pieces { self.last_piece_len } else { self.piece_length }
    }

    fn block_length(&self, pi: usize, bi: usize) -> u32 {
        let pl    = self.piece_len(pi);
        let begin = bi as u32 * BLOCK_SIZE;
        (pl - begin).min(BLOCK_SIZE)
    }

    /// Busca el primer bloque que satisface `predicate` en la pieza más rara
    /// que el peer tiene, marca el bloque como `InFlight(n+1)` y devuelve
    /// la tarea.
    fn find_block<F>(&mut self, peer_pieces: &[bool], predicate: F) -> Option<BlockTask>
    where
        F: Fn(&BlockState) -> bool,
    {
        // Selección rarest-first: encontrar la pieza candidata con menor availability.
        let mut best: Option<(usize, u32)> = None; // (pi, availability)

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

        // Primer bloque que cumple el predicado dentro de la pieza elegida.
        for bi in 0..self.block_state[pi].len() {
            if predicate(&self.block_state[pi][bi]) {
                let n = match self.block_state[pi][bi] {
                    BlockState::Pending     => 1,
                    BlockState::InFlight(n) => n + 1,
                    BlockState::Done        => continue,
                };
                self.block_state[pi][bi] = BlockState::InFlight(n);

                let begin  = bi as u32 * BLOCK_SIZE;
                let length = self.block_length(pi, bi);
                return Some(BlockTask { fi: self.fi, pi: pi as u32, bi: bi as u32, begin, length });
            }
        }

        None
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const PL: u32 = 4 * BLOCK_SIZE; // 64 KiB = 4 bloques

    fn sched(num_pieces: usize) -> BlockScheduler {
        BlockScheduler::new(0, num_pieces, PL, PL)
    }

    // ── Prioridad de scheduling ───────────────────────────────────────────────

    #[test]
    fn priority_pending_before_inflight() {
        let mut s = sched(2);
        // Pieza 0: todo disponible, availability = 1
        s.add_peer_bitfield(&[true, true]);
        let peer = vec![true, true];

        // Stream A → bloque (0,0): Pending → InFlight(1)
        let t1 = s.schedule(&peer).unwrap();
        assert_eq!((t1.pi, t1.bi), (0, 0));

        // Stream B → debe ir a otro Pending, no redundar en (0,0)
        let t2 = s.schedule(&peer).unwrap();
        // Debe ser bloque Pending: (0,1), (0,2), (0,3) o primer bloque de pieza 1
        assert!(
            t2.bi != 0 || t2.pi != 0,
            "debería elegir bloque Pending, no redundar en (0,0)"
        );
        assert_eq!(t2.pi, 0, "pieza 0 tiene bloques Pending");
        assert_eq!(t2.bi, 1);
    }

    #[test]
    fn fills_all_blocks_before_redundancy() {
        let mut s = sched(1); // 1 pieza, 4 bloques
        s.add_peer_bitfield(&[true]);
        let peer = vec![true];

        // 4 streams → cada uno recibe un bloque distinto (Pending)
        let tasks: Vec<_> = (0..4).map(|_| s.schedule(&peer).unwrap()).collect();
        let blocks: std::collections::HashSet<u32> = tasks.iter().map(|t| t.bi).collect();
        assert_eq!(blocks.len(), 4, "4 streams deben cubrir 4 bloques distintos");

        // 5.º stream → ya no hay Pending, debe ir a InFlight(1 < TYPICAL=2)
        let t5 = s.schedule(&peer).unwrap();
        // El bloque elegido debe estar ahora InFlight(2)
        match s.block_state[0][t5.bi as usize] {
            BlockState::InFlight(2) => {}
            other => panic!("esperado InFlight(2), got {other:?}"),
        }
    }

    #[test]
    fn max_streams_respected() {
        let mut s = sched(1); // 1 pieza, 4 bloques
        s.add_peer_bitfield(&[true]);
        let peer = vec![true];

        // Saturar bloque (0,0) al máximo
        for _ in 0..MAX_STREAMS_PER_BLOCK {
            // Consumimos bloques en orden; necesitamos forzar que schedule
            // vuelva al bloque 0 una vez los demás estén saturados.
            // Para este test simplemente vaciamos todos los Pending primero.
            let _ = s.schedule(&peer);
        }
        // Ahora todos los bloques están InFlight(1). Añadir TYPICAL-1 rondas más
        // hasta saturar el bloque 0 hasta MAX.
        // Marcamos bloques 1,2,3 como Done para que el scheduler solo tenga bloque 0.
        s.mark_block_done(0, 1);
        s.mark_block_done(0, 2);
        s.mark_block_done(0, 3);
        // Bloque 0 tiene InFlight(1). Añadir hasta MAX-1 streams más.
        for expected_n in 2..=MAX_STREAMS_PER_BLOCK {
            let t = s.schedule(&peer).unwrap();
            assert_eq!(t.bi, 0);
            match s.block_state[0][0] {
                BlockState::InFlight(n) => assert_eq!(n, expected_n),
                other => panic!("esperado InFlight({expected_n}), got {other:?}"),
            }
        }
        // Una vez en MAX, no se puede asignar más
        assert!(s.schedule(&peer).is_none(), "no debe asignar más allá de MAX");
    }

    #[test]
    fn rarest_first_ordering() {
        let mut s = sched(3);
        // Pieza 0: 2 peers, pieza 1: 1 peer, pieza 2: 3 peers
        s.add_peer_bitfield(&[true,  true,  true]);
        s.add_peer_bitfield(&[true,  false, true]);
        s.add_peer_bitfield(&[false, false, true]);
        // availability: [2, 1, 3]
        let peer = vec![true, true, true];
        let t = s.schedule(&peer).unwrap();
        assert_eq!(t.pi, 1, "pieza 1 es la más rara (availability=1)");
    }

    #[test]
    fn mark_block_done_triggers_piece_complete() {
        let mut s = sched(1); // 1 pieza, 4 bloques
        s.add_peer_bitfield(&[true]);
        let peer = vec![true];

        // Asignar y completar 3 de 4 bloques
        for bi in 0u32..3 {
            s.schedule(&peer);
            assert!(!s.mark_block_done(0, bi), "pieza no debe estar completa todavía");
        }
        // Asignar y completar el último bloque
        s.schedule(&peer);
        assert!(s.mark_block_done(0, 3), "pieza debe estar lista para verificación");

        // Segunda llamada no debe devolver true (ya está en verificación)
        assert!(!s.mark_block_done(0, 3), "doble verificación no permitida");
    }

    #[test]
    fn schedule_pending_only_returns_none_when_all_inflight() {
        let mut s = sched(1);
        s.add_peer_bitfield(&[true]);
        let peer = vec![true];

        // Asignar todos los bloques
        for _ in 0..4 { s.schedule(&peer); }
        // No hay Pending → schedule_pending debe devolver None
        assert!(s.schedule_pending(&peer).is_none());
    }

    #[test]
    fn mark_block_failed_returns_to_pending() {
        let mut s = sched(1);
        s.add_peer_bitfield(&[true]);
        let peer = vec![true];

        s.schedule(&peer); // bloque (0,0) → InFlight(1)
        s.mark_block_failed(0, 0); // → Pending
        assert_eq!(s.block_state[0][0], BlockState::Pending);

        // Debe poder volver a asignarse
        let t = s.schedule(&peer).unwrap();
        assert_eq!((t.pi, t.bi), (0, 0));
    }

    #[test]
    fn hash_failed_resets_all_blocks() {
        let mut s = sched(1);
        s.add_peer_bitfield(&[true]);
        let peer = vec![true];

        // Completar todos los bloques
        for bi in 0u32..4 { s.schedule(&peer); s.mark_block_done(0, bi); }
        assert!(s.piece_verifying[0]);

        s.mark_piece_hash_failed(0);
        assert!(!s.piece_verifying[0]);
        assert!(s.block_state[0].iter().all(|s| *s == BlockState::Pending));
    }

    #[test]
    fn is_complete_after_all_pieces_verified() {
        let mut s = sched(2);
        assert!(!s.is_complete());
        s.mark_piece_verified(0);
        assert!(!s.is_complete());
        s.mark_piece_verified(1);
        assert!(s.is_complete());
    }

    #[test]
    fn seeder_init_is_complete() {
        let mut s = sched(3);
        for pi in 0..3u32 { s.mark_piece_verified(pi); }
        assert!(s.is_complete());
        assert_eq!(s.progress(), 1.0);
    }

    #[test]
    fn skips_done_and_verifying_pieces() {
        let mut s = sched(3);
        s.add_peer_bitfield(&[true, true, true]);
        let peer = vec![true, true, true];

        s.mark_piece_verified(0);      // pieza 0 done
        s.piece_verifying[2] = true;   // pieza 2 en verificación (simular)

        // Solo debe ofrecer bloques de pieza 1
        let t = s.schedule(&peer).unwrap();
        assert_eq!(t.pi, 1);
    }
}
