#[cfg(test)]
mod tests {
    use bitturbulence_protocol::BLOCK_SIZE;
    use crate::scheduler::{BlockScheduler, types::{BlockState, MAX_STREAMS_PER_BLOCK}};

    const PL: u32 = 4 * BLOCK_SIZE; // 64 KiB = 4 bloques

    fn sched(num_pieces: usize) -> BlockScheduler {
        BlockScheduler::new(0, num_pieces, PL, PL)
    }

    // ── Prioridad de scheduling ───────────────────────────────────────────────

    #[test]
    fn priority_pending_before_inflight() {
        let mut s = sched(2);
        s.add_peer_bitfield(&[true, true]);
        let peer = vec![true, true];

        let t1 = s.schedule(&peer).unwrap();
        assert_eq!((t1.pi, t1.bi), (0, 0));

        let t2 = s.schedule(&peer).unwrap();
        assert!(
            t2.bi != 0 || t2.pi != 0,
            "debería elegir bloque Pending, no redundar en (0,0)"
        );
        assert_eq!(t2.pi, 0, "pieza 0 tiene bloques Pending");
        assert_eq!(t2.bi, 1);
    }

    #[test]
    fn fills_all_blocks_before_redundancy() {
        let mut s = sched(1);
        s.add_peer_bitfield(&[true]);
        let peer = vec![true];

        let tasks: Vec<_> = (0..4).map(|_| s.schedule(&peer).unwrap()).collect();
        let blocks: std::collections::HashSet<u32> = tasks.iter().map(|t| t.bi).collect();
        assert_eq!(blocks.len(), 4, "4 streams deben cubrir 4 bloques distintos");

        let t5 = s.schedule(&peer).unwrap();
        match s.block_state[0][t5.bi as usize] {
            BlockState::InFlight(2) => {}
            other => panic!("esperado InFlight(2), got {other:?}"),
        }
    }

    #[test]
    fn max_streams_respected() {
        let mut s = sched(1);
        s.add_peer_bitfield(&[true]);
        let peer = vec![true];

        for _ in 0..MAX_STREAMS_PER_BLOCK { let _ = s.schedule(&peer); }
        s.mark_block_done(0, 1, [0u8; 32]);
        s.mark_block_done(0, 2, [0u8; 32]);
        s.mark_block_done(0, 3, [0u8; 32]);

        for expected_n in 2..=MAX_STREAMS_PER_BLOCK {
            let t = s.schedule(&peer).unwrap();
            assert_eq!(t.bi, 0);
            match s.block_state[0][0] {
                BlockState::InFlight(n) => assert_eq!(n, expected_n),
                other => panic!("esperado InFlight({expected_n}), got {other:?}"),
            }
        }
        assert!(s.schedule(&peer).is_none(), "no debe asignar más allá de MAX");
    }

    #[test]
    fn rarest_first_ordering() {
        let mut s = sched(3);
        s.add_peer_bitfield(&[true,  true,  true]);
        s.add_peer_bitfield(&[true,  false, true]);
        s.add_peer_bitfield(&[false, false, true]);
        let peer = vec![true, true, true];
        let t = s.schedule(&peer).unwrap();
        assert_eq!(t.pi, 1, "pieza 1 es la más rara (availability=1)");
    }

    #[test]
    fn mark_block_done_triggers_piece_complete() {
        let mut s = sched(1);
        s.add_peer_bitfield(&[true]);
        let peer = vec![true];

        for bi in 0u32..3 {
            s.schedule(&peer);
            assert!(!s.mark_block_done(0, bi, [0u8; 32]));
        }
        s.schedule(&peer);
        assert!(s.mark_block_done(0, 3, [0u8; 32]));
        assert!(!s.mark_block_done(0, 3, [0u8; 32]));
    }

    #[test]
    fn schedule_pending_only_returns_none_when_all_inflight() {
        let mut s = sched(1);
        s.add_peer_bitfield(&[true]);
        let peer = vec![true];

        for _ in 0..4 { s.schedule(&peer); }
        assert!(s.schedule_pending(&peer).is_none());
    }

    #[test]
    fn mark_block_failed_returns_to_pending() {
        let mut s = sched(1);
        s.add_peer_bitfield(&[true]);
        let peer = vec![true];

        s.schedule(&peer);
        s.mark_block_failed(0, 0);
        assert_eq!(s.block_state[0][0], BlockState::Pending);

        let t = s.schedule(&peer).unwrap();
        assert_eq!((t.pi, t.bi), (0, 0));
    }

    #[test]
    fn hash_failed_resets_all_blocks() {
        let mut s = sched(1);
        s.add_peer_bitfield(&[true]);
        let peer = vec![true];

        for bi in 0u32..4 { s.schedule(&peer); s.mark_block_done(0, bi, [0u8; 32]); }
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

        s.mark_piece_verified(0);
        s.piece_verifying[2] = true;

        let t = s.schedule(&peer).unwrap();
        assert_eq!(t.pi, 1);
    }
}
