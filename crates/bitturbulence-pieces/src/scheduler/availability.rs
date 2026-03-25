use super::BlockScheduler;

impl BlockScheduler {
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
}
