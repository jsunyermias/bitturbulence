use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    pub file_index:  u16,
    pub piece_index: u32,
    pub begin:       u32,
    pub length:      u32,
}

/// Estado de disponibilidad de piezas de un archivo remoto.
#[derive(Debug, Clone)]
pub enum FileAvailability {
    Unknown,
    HaveAll,
    HaveNone,
    Bitmap(Vec<bool>),
}

impl FileAvailability {
    pub fn has_piece(&self, index: u32) -> bool {
        match self {
            Self::HaveAll      => true,
            Self::HaveNone     => false,
            Self::Unknown      => false,
            Self::Bitmap(bits) => bits.get(index as usize).copied().unwrap_or(false),
        }
    }
}

/// Estado de la sesión con un peer.
#[derive(Debug, Clone)]
pub struct SessionState {
    /// Disponibilidad por file_index.
    pub files: HashMap<u16, FileAvailability>,
    /// Número de archivos del torrent.
    pub num_files: usize,
    /// Requests en vuelo.
    pub pending: Vec<PendingRequest>,
}

impl SessionState {
    pub fn new(num_files: usize) -> Self {
        Self {
            files: HashMap::new(),
            num_files,
            pending: Vec::new(),
        }
    }

    pub fn apply_have_all(&mut self, file_index: u16) {
        self.files.insert(file_index, FileAvailability::HaveAll);
    }

    pub fn apply_have_none(&mut self, file_index: u16) {
        self.files.insert(file_index, FileAvailability::HaveNone);
    }

    pub fn apply_have_piece(&mut self, file_index: u16, piece_index: u32) {
        let entry = self.files
            .entry(file_index)
            .or_insert(FileAvailability::HaveNone);
        match entry {
            FileAvailability::HaveAll => {}
            FileAvailability::Bitmap(bits) => {
                let idx = piece_index as usize;
                if idx >= bits.len() { bits.resize(idx + 1, false); }
                bits[idx] = true;
            }
            _ => {
                let mut bits = vec![false; piece_index as usize + 1];
                bits[piece_index as usize] = true;
                *entry = FileAvailability::Bitmap(bits);
            }
        }
    }

    pub fn apply_have_bitmap(&mut self, file_index: u16, bitmap: &[u8]) {
        let mut bits = Vec::with_capacity(bitmap.len() * 8);
        for byte in bitmap {
            for bit in (0..8).rev() {
                bits.push((byte >> bit) & 1 == 1);
            }
        }
        self.files.insert(file_index, FileAvailability::Bitmap(bits));
    }

    pub fn peer_has_piece(&self, file_index: u16, piece_index: u32) -> bool {
        self.files
            .get(&file_index)
            .map(|a| a.has_piece(piece_index))
            .unwrap_or(false)
    }

    pub fn add_pending(&mut self, req: PendingRequest) {
        self.pending.push(req);
    }

    pub fn remove_pending(&mut self, file_index: u16, piece_index: u32, begin: u32) {
        self.pending.retain(|r| {
            !(r.file_index == file_index && r.piece_index == piece_index && r.begin == begin)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn have_all_has_any_piece() {
        let mut s = SessionState::new(1);
        s.apply_have_all(0);
        assert!(s.peer_has_piece(0, 0));
        assert!(s.peer_has_piece(0, 9999));
    }

    #[test]
    fn have_none_has_no_piece() {
        let mut s = SessionState::new(1);
        s.apply_have_none(0);
        assert!(!s.peer_has_piece(0, 0));
    }

    #[test]
    fn have_piece_updates_bitmap() {
        let mut s = SessionState::new(1);
        s.apply_have_piece(0, 5);
        assert!(s.peer_has_piece(0, 5));
        assert!(!s.peer_has_piece(0, 4));
    }

    #[test]
    fn bitmap_parsed_correctly() {
        let mut s = SessionState::new(1);
        // 0b1010_0000 → piezas 0 y 2
        s.apply_have_bitmap(0, &[0b1010_0000]);
        assert!(s.peer_has_piece(0, 0));
        assert!(!s.peer_has_piece(0, 1));
        assert!(s.peer_has_piece(0, 2));
        assert!(!s.peer_has_piece(0, 3));
    }

    #[test]
    fn pending_add_and_remove() {
        let mut s = SessionState::new(1);
        s.add_pending(PendingRequest { file_index: 0, piece_index: 3, begin: 0, length: 4096 });
        assert_eq!(s.pending.len(), 1);
        s.remove_pending(0, 3, 0);
        assert!(s.pending.is_empty());
    }
}
