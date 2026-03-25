use sha2::{Digest, Sha256};
use crate::priority::Priority;

/// Raíz Merkle de un slice de hashes de 32 bytes.
/// Rellena a la siguiente potencia de 2 con `[0u8; 32]` (igual que BT v2).
fn merkle_root_of_slices(hashes: &[[u8; 32]]) -> [u8; 32] {
    if hashes.is_empty() { return [0u8; 32]; }
    if hashes.len() == 1 { return hashes[0]; }
    let n = hashes.len().next_power_of_two();
    let mut layer: Vec<[u8; 32]> = Vec::with_capacity(n);
    layer.extend_from_slice(hashes);
    layer.resize(n, [0u8; 32]);
    while layer.len() > 1 {
        layer = layer.chunks(2).map(|p| {
            let mut h = Sha256::new();
            h.update(p[0]);
            h.update(p[1]);
            h.finalize().into()
        }).collect();
    }
    layer[0]
}

/// Tamaño fijo de bloque de transferencia: 16 KiB.
///
/// Un bloque es la unidad atómica de Request/Piece en el wire.
/// Una pieza es un grupo de bloques y es la unidad de verificación SHA-256.
pub const BLOCK_SIZE: u32 = 16 * 1024;

/// Calcula el tamaño de pieza óptimo para un archivo dado su tamaño en bytes.
///
/// El resultado es siempre potencia de 2 y múltiplo de [`BLOCK_SIZE`], acotado
/// entre 64 KiB (4 bloques) y 1 GiB (65 536 bloques).
///
/// Fórmula: `next_pow2(file_size / 2048)`, acotado a [64 KiB, 512 MiB],
/// con caso especial ≥ 1 TiB → 1 GiB. Produce ~2048 piezas en cada límite.
///
/// Tabla de límites:
///   ≤  128 MB  →   64 KB  (    4 bloques)
///   ≤  256 MB  →  128 KB  (    8 bloques)
///   ≤  512 MB  →  256 KB  (   16 bloques)
///   ≤    1 GB  →  512 KB  (   32 bloques)
///   ≤    2 GB  →    1 MB  (   64 bloques)
///   ≤    4 GB  →    2 MB  (  128 bloques)
///   ≤    8 GB  →    4 MB  (  256 bloques)
///   ≤   16 GB  →    8 MB  (  512 bloques)
///   ≤   32 GB  →   16 MB  ( 1024 bloques)
///   ≤   64 GB  →   32 MB  ( 2048 bloques)
///   ≤  128 GB  →   64 MB  ( 4096 bloques)
///   ≤  256 GB  →  128 MB  ( 8192 bloques)
///   ≤  512 GB  →  256 MB  (16384 bloques)
///   <    1 TB  →  512 MB  (32768 bloques)
///   ≥    1 TB  →    1 GB  (65536 bloques)
pub fn piece_length_for_size(file_size: u64) -> u32 {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;

    const MIN_PIECE: u64 = 64 * KB;  // 4 bloques
    const MAX_PIECE: u64 = GB;       // 65536 bloques

    if file_size == 0 { return MIN_PIECE as u32; }
    if file_size >= TB { return MAX_PIECE as u32; }

    // next_pow2(file_size / 2048), acotado a [64 KB, 512 MB].
    let raw = (file_size / 2048).max(1);
    let piece = raw.next_power_of_two().clamp(MIN_PIECE, MAX_PIECE / 2);

    // No puede superar el tamaño del archivo.
    piece.min(file_size) as u32
}

/// Número de piezas para un archivo dado su tamaño y el tamaño de pieza.
pub fn num_pieces(file_size: u64, piece_length: u32) -> u32 {
    if file_size == 0 { return 0; }
    file_size.div_ceil(piece_length as u64) as u32
}

/// Descripción de un archivo dentro del torrent.
///
/// `piece_length` y `num_pieces` son propiedades computadas, no almacenadas,
/// porque son deterministas a partir de `size` y `piece_hashes.len()`.
/// Esto evita inconsistencias en el formato serializado.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileEntry {
    /// Ruta relativa al directorio raíz del torrent.
    pub path: Vec<String>,
    /// Tamaño en bytes.
    pub size: u64,
    /// Raíces Merkle de cada pieza (32 bytes × num_pieces).
    ///
    /// Cada entrada es la raíz Merkle de los hashes SHA-256 de los bloques (16 KiB)
    /// que forman esa pieza — igual que en BitTorrent v2.
    /// NO es SHA-256 de los datos de la pieza concatenados.
    pub piece_hashes: Vec<[u8; 32]>,
    /// Prioridad inicial de descarga.
    pub priority: Priority,
}

impl FileEntry {
    pub fn new(path: Vec<String>, size: u64, priority: Priority) -> Self {
        let n = num_pieces(size, piece_length_for_size(size)) as usize;
        Self {
            path,
            size,
            piece_hashes: vec![[0u8; 32]; n],
            priority,
        }
    }

    /// Tamaño de pieza calculado automáticamente a partir del tamaño del archivo.
    pub fn piece_length(&self) -> u32 {
        piece_length_for_size(self.size)
    }

    /// Número de piezas, derivado de la longitud del vector de hashes.
    pub fn num_pieces(&self) -> u32 {
        self.piece_hashes.len() as u32
    }

    /// Longitud real de la última pieza (puede ser menor que piece_length).
    pub fn last_piece_length(&self) -> u32 {
        if self.num_pieces() == 0 { return 0; }
        let rem = (self.size % self.piece_length() as u64) as u32;
        if rem == 0 { self.piece_length() } else { rem }
    }

    /// Raíz Merkle del archivo: raíz Merkle de todas las raíces de pieza.
    ///
    /// Equivale al "file merkle root" de BitTorrent v2.
    /// Se rellena a la siguiente potencia de 2 con hashes cero.
    pub fn file_root(&self) -> [u8; 32] {
        merkle_root_of_slices(&self.piece_hashes)
    }

    /// Longitud de la pieza `index` (la última puede ser más corta).
    pub fn piece_len(&self, index: u32) -> u32 {
        if index + 1 == self.num_pieces() {
            self.last_piece_length()
        } else {
            self.piece_length()
        }
    }
}

/// Metainfo completo de un archivo BitTurbulence.
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
        self.files.iter().map(|f| f.num_pieces() as u64).sum()
    }

    /// Calcula el info_hash a partir de los metadatos actuales.
    ///
    /// `info_hash = SHA-256(file_root_0 || file_root_1 || ...)`
    ///
    /// Donde cada `file_root` es la raíz Merkle de las raíces de pieza del
    /// archivo correspondiente. Para un torrent de un solo archivo es
    /// `SHA-256(file_root_0)`.
    pub fn compute_info_hash(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        for f in &self.files {
            h.update(f.file_root());
        }
        h.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    #[test]
    fn block_size_is_16k() {
        assert_eq!(BLOCK_SIZE, 16 * 1024);
    }

    #[test]
    fn piece_length_ranges() {
        // Mínimo: 64 KB (4 bloques de 16 KB)
        assert_eq!(piece_length_for_size(0),          64 * KB as u32);  // caso especial
        assert_eq!(piece_length_for_size(KB),          KB as u32);       // < 64 KB → capped al tamaño
        assert_eq!(piece_length_for_size(64 * KB),    64 * KB as u32);  // ≤ 128 MB → 64 KB

        // ~2048 piezas en cada límite de tramo
        assert_eq!(piece_length_for_size(128 * MB),   64 * KB as u32);
        assert_eq!(piece_length_for_size(256 * MB),  128 * KB as u32);
        assert_eq!(piece_length_for_size(512 * MB),  256 * KB as u32);
        assert_eq!(piece_length_for_size(GB),         512 * KB as u32);
        assert_eq!(piece_length_for_size(2   * GB),    1 * MB as u32);
        assert_eq!(piece_length_for_size(16  * GB),    8 * MB as u32);
        assert_eq!(piece_length_for_size(128 * GB),   64 * MB as u32);
        assert_eq!(piece_length_for_size(512 * GB),  256 * MB as u32);

        // Cap: ≥ 1 TB → 1 GB (65536 bloques)
        assert_eq!(piece_length_for_size(1024 * GB), 1024 * MB as u32);
    }

    #[test]
    fn piece_length_always_multiple_of_block_size() {
        for size in [1u64, KB, 64*KB, MB, 256*MB, GB, 16*GB, 1024*GB] {
            let pl = piece_length_for_size(size);
            // Para archivos >= BLOCK_SIZE el resultado debe ser múltiplo de BLOCK_SIZE.
            if size >= BLOCK_SIZE as u64 {
                assert_eq!(pl % BLOCK_SIZE, 0, "size={size} pl={pl} no es múltiplo de BLOCK_SIZE");
            }
        }
    }

    #[test]
    fn piece_length_never_exceeds_file_size() {
        for size in [1u64, 100, 999, 16 * KB - 1] {
            let pl = piece_length_for_size(size);
            assert!(pl as u64 <= size, "size={size} pl={pl}");
        }
    }

    #[test]
    fn num_pieces_correct() {
        let pl = 64 * 1024u32; // 64 KB
        assert_eq!(num_pieces(0, pl), 0);
        assert_eq!(num_pieces(pl as u64, pl), 1);
        assert_eq!(num_pieces(pl as u64 + 1, pl), 2);
        assert_eq!(num_pieces(2 * pl as u64, pl), 2);
    }

    #[test]
    fn last_piece_length_correct() {
        // Archivo de 5 MB (≤ 128 MB): piece_length = 64 KB → 80 piezas exactas
        let size = 5 * MB as u64;
        let f = FileEntry::new(vec!["a.bin".into()], size, Priority::Normal);
        assert_eq!(f.piece_length(), 64 * KB as u32);
        assert_eq!(f.num_pieces(), 80);
        assert_eq!(f.last_piece_length(), 64 * KB as u32); // divisible exacto
    }

    #[test]
    fn last_piece_shorter_when_not_multiple() {
        // 5 MB + 1 byte: 81 piezas, la última de 1 byte
        let size = 5 * MB as u64 + 1;
        let f = FileEntry::new(vec!["a.bin".into()], size, Priority::Normal);
        assert_eq!(f.num_pieces(), 81);
        assert_eq!(f.last_piece_length(), 1);
    }

    #[test]
    fn file_exact_multiple_of_piece() {
        let size = 256 * KB as u64; // exactamente 1 pieza
        let f = FileEntry::new(vec!["a.bin".into()], size, Priority::Normal);
        assert_eq!(f.last_piece_length(), f.piece_length());
    }
}
