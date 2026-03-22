/// Estrategia de selección de piezas.
/// Implementa rarest-first: prioriza las piezas que menos peers tienen.
pub struct PiecePicker {
    num_pieces: usize,
    /// Piezas que ya tenemos completas y verificadas.
    have: Vec<bool>,
    /// Piezas que estamos descargando actualmente.
    in_progress: Vec<bool>,
    /// Conteo de cuántos peers tienen cada pieza.
    availability: Vec<u32>,
}

impl PiecePicker {
    pub fn new(num_pieces: usize) -> Self {
        Self {
            num_pieces,
            have: vec![false; num_pieces],
            in_progress: vec![false; num_pieces],
            availability: vec![0; num_pieces],
        }
    }

    /// Registra el bitfield de un peer nuevo.
    pub fn add_peer_bitfield(&mut self, bitfield: &[bool]) {
        for (i, &has) in bitfield.iter().enumerate() {
            if i < self.num_pieces && has {
                self.availability[i] += 1;
            }
        }
    }

    /// Un peer anuncia que tiene la pieza `index`.
    pub fn add_peer_have(&mut self, index: usize) {
        if index < self.num_pieces {
            self.availability[index] += 1;
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

    /// Marca una pieza como completada y verificada.
    pub fn mark_complete(&mut self, index: usize) {
        if index < self.num_pieces {
            self.have[index] = true;
            self.in_progress[index] = false;
        }
    }

    /// Marca una pieza como fallida — vuelve al pool.
    pub fn mark_failed(&mut self, index: usize) {
        if index < self.num_pieces {
            self.in_progress[index] = false;
        }
    }

    /// Selecciona la siguiente pieza a descargar usando rarest-first.
    /// Solo devuelve piezas disponibles en el peer dado (su bitfield).
    pub fn pick(&mut self, peer_pieces: &[bool]) -> Option<usize> {
        let mut best: Option<(usize, u32)> = None; // (index, availability)

        for i in 0..self.num_pieces {
            if self.have[i] || self.in_progress[i] {
                continue;
            }
            let peer_has = peer_pieces.get(i).copied().unwrap_or(false);
            if !peer_has {
                continue;
            }
            let avail = self.availability[i];
            match best {
                None => best = Some((i, avail)),
                Some((_, best_avail)) if avail < best_avail => best = Some((i, avail)),
                _ => {}
            }
        }

        if let Some((index, _)) = best {
            self.in_progress[index] = true;
            Some(index)
        } else {
            None
        }
    }

    pub fn is_complete(&self) -> bool {
        self.have.iter().all(|&h| h)
    }

    pub fn num_complete(&self) -> usize {
        self.have.iter().filter(|&&h| h).count()
    }

    pub fn progress(&self) -> f32 {
        self.num_complete() as f32 / self.num_pieces as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_rarest_piece() {
        let mut picker = PiecePicker::new(4);
        // peer A tiene piezas 0,1,2,3
        picker.add_peer_bitfield(&[true, true, true, true]);
        // peer B solo tiene pieza 2
        picker.add_peer_bitfield(&[false, false, true, false]);

        // availability: [1,1,2,1]
        // peer local tiene [true,true,true,true]
        // rarest = pieza 0 (o 1 o 3, todas con avail=1, elige la primera)
        let peer = vec![true, true, true, true];
        let pick = picker.pick(&peer).unwrap();
        assert_ne!(pick, 2); // pieza 2 no es la más rara
    }

    #[test]
    fn skips_completed_pieces() {
        let mut picker = PiecePicker::new(3);
        picker.add_peer_bitfield(&[true, true, true]);
        picker.mark_complete(0);
        picker.mark_complete(2);

        let peer = vec![true, true, true];
        let pick = picker.pick(&peer).unwrap();
        assert_eq!(pick, 1);
    }

    #[test]
    fn returns_none_when_peer_has_nothing_we_need() {
        let mut picker = PiecePicker::new(3);
        picker.add_peer_bitfield(&[true, true, true]);
        picker.mark_complete(0);
        picker.mark_complete(1);
        picker.mark_complete(2);

        let peer = vec![true, true, true];
        assert!(picker.pick(&peer).is_none());
    }

    #[test]
    fn is_complete_when_all_done() {
        let mut picker = PiecePicker::new(2);
        assert!(!picker.is_complete());
        picker.mark_complete(0);
        picker.mark_complete(1);
        assert!(picker.is_complete());
    }
}
