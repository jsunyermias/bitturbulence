use crate::priority::Priority;

/// Calcula el tamaño de pieza óptimo para un archivo dado su tamaño en bytes.
/// Siempre potencia de 2. Nunca mayor que el tamaño del archivo.
///
/// Tabla de rangos:
///   <      1 MB  →     4 KB
///   <      8 MB  →    16 KB
///   <     64 MB  →    64 KB
///   <    512 MB  →   256 KB
///   <      4 GB  →     1 MB
///   <     32 GB  →     4 MB
///   <    256 GB  →    16 MB
///   <    512 GB  →    64 MB
///   >=   512 GB  →   256 MB
pub fn piece_length_for_size(file_size: u64) -> u32 {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    let piece_len = match file_size {
        s if s <    1 * MB             => 4  * KB,
        s if s <    8 * MB             => 16 * KB,
        s if s <   64 * MB             => 64 * KB,
        s if s <  512 * MB             => 256 * KB,
        s if s <    4 * GB             => 1  * MB,
        s if s <   32 * GB             => 4  * MB,
        s if s <  256 * GB             => 16 * MB,
        s if s <  512 * GB             => 64 * MB,
        _                              => 256 * MB,
    };
    // No puede ser mayor que el archivo
    if file_size == 0 { return 4 * KB as u32; }
    piece_len.min(file_size) as u32
}

/// Número de piezas para un archivo dado su tamaño y el tamaño de pieza.
pub fn num_pieces(file_size: u64, piece_length: u32) -> u32 {
    if file_size == 0 { return 0; }
    ((file_size + piece_length as u64 - 1) / piece_length as u64) as u32
}

/// Descripción de un archivo dentro del torrent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileEntry {
    /// Ruta relativa al directorio raíz del torrent.
    pub path: Vec<String>,
    /// Tamaño en bytes.
    pub size: u64,
    /// Tamaño de pieza calculado automáticamente.
    pub piece_length: u32,
    /// Número de piezas.
    pub num_pieces: u32,
    /// Hashes SHA-256 de cada pieza (32 bytes × num_pieces).
    pub piece_hashes: Vec<[u8; 32]>,
    /// Prioridad inicial de descarga.
    pub priority: Priority,
}

impl FileEntry {
    pub fn new(path: Vec<String>, size: u64, priority: Priority) -> Self {
        let piece_length = piece_length_for_size(size);
        let n = num_pieces(size, piece_length);
        Self {
            path,
            size,
            piece_length,
            num_pieces: n,
            piece_hashes: vec![[0u8; 32]; n as usize],
            priority,
        }
    }

    /// Longitud real de la última pieza (puede ser menor que piece_length).
    pub fn last_piece_length(&self) -> u32 {
        if self.num_pieces == 0 { return 0; }
        let rem = (self.size % self.piece_length as u64) as u32;
        if rem == 0 { self.piece_length } else { rem }
    }

    /// Longitud de la pieza `index` (la última puede ser más corta).
    pub fn piece_len(&self, index: u32) -> u32 {
        if index + 1 == self.num_pieces {
            self.last_piece_length()
        } else {
            self.piece_length
        }
    }
}

/// Metainfo completo del torrent quictorrent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Metainfo {
    /// Nombre del torrent (directorio raíz para multi-file).
    pub name: String,
    /// Info hash SHA-256 del metainfo serializado.
    pub info_hash: [u8; 32],
    /// Lista de archivos. Cada uno tiene su propio piece_length.
    pub files: Vec<FileEntry>,
    /// Trackers opcionales.
    pub trackers: Vec<String>,
    /// Comentario opcional.
    pub comment: Option<String>,
}

impl Metainfo {
    /// Tamaño total de todos los archivos.
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// Número total de piezas en todos los archivos.
    pub fn total_pieces(&self) -> u64 {
        self.files.iter().map(|f| f.num_pieces as u64).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piece_length_ranges() {
        assert_eq!(piece_length_for_size(0),                    4 * 1024);
        assert_eq!(piece_length_for_size(512 * 1024),           4 * 1024);
        assert_eq!(piece_length_for_size(1024 * 1024),         16 * 1024);
        assert_eq!(piece_length_for_size(8 * 1024 * 1024),     64 * 1024);
        assert_eq!(piece_length_for_size(64 * 1024 * 1024),   256 * 1024);
        assert_eq!(piece_length_for_size(512 * 1024 * 1024), 1024 * 1024);
        assert_eq!(piece_length_for_size(4  * 1024 * 1024 * 1024),  4 * 1024 * 1024);
    }

    #[test]
    fn piece_length_never_exceeds_file_size() {
        for size in [1u64, 100, 999, 1023] {
            let pl = piece_length_for_size(size);
            assert!(pl as u64 <= size, "size={} pl={}", size, pl);
        }
    }

    #[test]
    fn num_pieces_correct() {
        assert_eq!(num_pieces(0, 4096), 0);
        assert_eq!(num_pieces(4096, 4096), 1);
        assert_eq!(num_pieces(4097, 4096), 2);
        assert_eq!(num_pieces(8192, 4096), 2);
    }

    #[test]
    fn last_piece_length_correct() {
        let f = FileEntry::new(vec!["a.bin".into()], 5000, Priority::Normal);
        assert_eq!(f.piece_length, 4096);
        assert_eq!(f.num_pieces, 2);
        assert_eq!(f.last_piece_length(), 5000 - 4096);
    }

    #[test]
    fn file_exact_multiple_of_piece() {
        let f = FileEntry::new(vec!["a.bin".into()], 8192, Priority::Normal);
        assert_eq!(f.last_piece_length(), f.piece_length);
    }
}
