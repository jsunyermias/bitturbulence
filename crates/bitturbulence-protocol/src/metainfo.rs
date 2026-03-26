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

    // ── BitFlow de 13 archivos con propiedades variadas ───────────────────────

    /// Los 13 archivos del flujo de prueba, junto con sus propiedades esperadas.
    ///
    /// | fi | ruta                            | tamaño          | prioridad |
    /// |----|---------------------------------|-----------------|-----------|
    /// |  0 | readme.txt                      | 1 B             | Maximum   |
    /// |  1 | docs/CHANGELOG.md               | 100 B           | VeryHigh  |
    /// |  2 | config.toml                     | 512 B           | Higher    |
    /// |  3 | assets/icon.png                 | 16 KB           | High      |
    /// |  4 | data/sample.bin                 | 64 KB (1 pieza) | Normal    |
    /// |  5 | cache/index.bin                 | 64 KB + 1 B     | Low       |
    /// |  6 | src/bundle.js                   | 1 MB            | Lower     |
    /// |  7 | dist/app.wasm                   | 5 MB            | VeryLow   |
    /// |  8 | dist/app.wasm.map               | 5 MB + 999 B    | Minimum   |
    /// |  9 | video/intro.mp4                 | 128 MB          | Maximum   |
    /// | 10 | video/main.mp4                  | 256 MB + 1 B    | VeryHigh  |
    /// | 11 | backup/snapshot.tar             | 1 GB            | Higher    |
    /// | 12 | media/archive/full.tar.gz       | 2 GB            | High      |
    fn make_13_file_meta() -> Metainfo {
        use Priority::*;

        let specs: &[(&[&str], u64, Priority)] = &[
            (&["readme.txt"],                          1,              Maximum),
            (&["docs", "CHANGELOG.md"],                100,            VeryHigh),
            (&["config.toml"],                         512,            Higher),
            (&["assets", "icon.png"],                  16 * KB,        High),
            (&["data", "sample.bin"],                  64 * KB,        Normal),
            (&["cache", "index.bin"],                  64 * KB + 1,    Low),
            (&["src", "bundle.js"],                    MB,             Lower),
            (&["dist", "app.wasm"],                    5 * MB,         VeryLow),
            (&["dist", "app.wasm.map"],                5 * MB + 999,   Minimum),
            (&["video", "intro.mp4"],                  128 * MB,       Maximum),
            (&["video", "main.mp4"],                   256 * MB + 1,   VeryHigh),
            (&["backup", "snapshot.tar"],              GB,             Higher),
            (&["media", "archive", "full.tar.gz"],     2 * GB,         High),
        ];

        let files: Vec<FileEntry> = specs.iter().map(|(path, size, prio)| {
            FileEntry::new(path.iter().map(|s| s.to_string()).collect(), *size, *prio)
        }).collect();

        let mut meta = Metainfo {
            name:      "test-multifile-flow".into(),
            info_hash: [0u8; 32],
            files,
            trackers:  vec![
                "https://tracker.example.com/announce".into(),
                "udp://tracker2.example.com:6969/announce".into(),
            ],
            comment: Some("BitFlow de prueba: 13 archivos con propiedades variadas".into()),
        };
        meta.info_hash = meta.compute_info_hash();
        meta
    }

    #[test]
    fn multifile_13_count() {
        assert_eq!(make_13_file_meta().files.len(), 13);
    }

    #[test]
    fn multifile_13_all_nine_priorities_present() {
        use std::collections::HashSet;
        let meta = make_13_file_meta();
        let unique: HashSet<u8> = meta.files.iter().map(|f| f.priority.as_u8()).collect();
        assert_eq!(unique.len(), 9, "deben estar los 9 niveles de prioridad");
    }

    #[test]
    fn multifile_13_priority_per_file() {
        use Priority::*;
        let meta = make_13_file_meta();
        let expected = [
            Maximum, VeryHigh, Higher, High, Normal,
            Low, Lower, VeryLow, Minimum,
            Maximum, VeryHigh, Higher, High,
        ];
        for (fi, (&exp, file)) in expected.iter().zip(meta.files.iter()).enumerate() {
            assert_eq!(file.priority, exp, "fi={fi}");
        }
    }

    #[test]
    fn multifile_13_total_size() {
        let expected = 1 + 100 + 512
            + 16 * KB + 64 * KB + (64 * KB + 1)
            + MB + 5 * MB + (5 * MB + 999)
            + 128 * MB + (256 * MB + 1) + GB + 2 * GB;
        assert_eq!(make_13_file_meta().total_size(), expected);
    }

    #[test]
    fn multifile_13_piece_lengths() {
        let meta = make_13_file_meta();
        // assets/icon.png: 16 KB < 64 KB mínimo → piece_length capado al tamaño
        assert_eq!(meta.files[3].piece_length(), 16 * KB as u32);
        // data/sample.bin … video/intro.mp4 (64 KB – 128 MB) → piece_length = 64 KB
        for fi in 4..=9 {
            assert_eq!(meta.files[fi].piece_length(), 64 * KB as u32, "fi={fi}");
        }
        // video/main.mp4: 256 MB + 1 B → 128 KB
        assert_eq!(meta.files[10].piece_length(), 128 * KB as u32);
        // backup/snapshot.tar: 1 GB → 512 KB
        assert_eq!(meta.files[11].piece_length(), 512 * KB as u32);
        // media/archive/full.tar.gz: 2 GB → 1 MB
        assert_eq!(meta.files[12].piece_length(), MB as u32);
    }

    #[test]
    fn multifile_13_last_piece_lengths() {
        let meta = make_13_file_meta();
        // data/sample.bin: 64 KB exacto → última pieza = piece_length
        assert_eq!(meta.files[4].last_piece_length(), 64 * KB as u32);
        // cache/index.bin: 64 KB + 1 B → última pieza de 1 B
        assert_eq!(meta.files[5].last_piece_length(), 1);
        // dist/app.wasm.map: 5 MB + 999 B → última pieza de 999 B
        assert_eq!(meta.files[8].last_piece_length(), 999);
        // video/main.mp4: 256 MB + 1 B, piece=128 KB → última pieza de 1 B
        assert_eq!(meta.files[10].last_piece_length(), 1);
    }

    #[test]
    fn multifile_13_total_pieces_matches_sum() {
        let meta = make_13_file_meta();
        let sum: u64 = meta.files.iter().map(|f| f.num_pieces() as u64).sum();
        assert_eq!(meta.total_pieces(), sum);
    }

    #[test]
    fn multifile_13_num_pieces_per_file() {
        let meta = make_13_file_meta();
        // data/sample.bin: 64 KB / 64 KB = 1 pieza
        assert_eq!(meta.files[4].num_pieces(), 1);
        // cache/index.bin: ceil((64 KB + 1) / 64 KB) = 2 piezas
        assert_eq!(meta.files[5].num_pieces(), 2);
        // dist/app.wasm: ceil(5 MB / 64 KB) = 80 piezas
        assert_eq!(meta.files[7].num_pieces(), 80);
        // dist/app.wasm.map: ceil((5 MB + 999) / 64 KB) = 81 piezas
        assert_eq!(meta.files[8].num_pieces(), 81);
        // video/main.mp4: ceil((256 MB + 1) / 128 KB) = 2049 piezas
        assert_eq!(meta.files[10].num_pieces(), 2049);
    }

    #[test]
    fn multifile_13_info_hash_deterministic() {
        let m1 = make_13_file_meta();
        let m2 = make_13_file_meta();
        assert_eq!(m1.info_hash, m2.info_hash);
        assert_ne!(m1.info_hash, [0u8; 32]);
    }

    #[test]
    fn multifile_13_info_hash_changes_on_modification() {
        let mut meta = make_13_file_meta();
        let original = meta.info_hash;
        meta.files[4].piece_hashes[0] = [0xFFu8; 32];
        meta.info_hash = meta.compute_info_hash();
        assert_ne!(meta.info_hash, original);
    }

    #[test]
    fn multifile_13_nested_paths_preserved() {
        let meta = make_13_file_meta();
        assert_eq!(meta.files[1].path,  vec!["docs", "CHANGELOG.md"]);
        assert_eq!(meta.files[10].path, vec!["video", "main.mp4"]);
        assert_eq!(meta.files[12].path, vec!["media", "archive", "full.tar.gz"]);
    }

    #[test]
    fn multifile_13_serialization_roundtrip() {
        let meta = make_13_file_meta();
        let json = serde_json::to_string_pretty(&meta).expect("serializar");
        let restored: Metainfo = serde_json::from_str(&json).expect("deserializar");

        assert_eq!(restored.name,      meta.name);
        assert_eq!(restored.info_hash, meta.info_hash);
        assert_eq!(restored.trackers,  meta.trackers);
        assert_eq!(restored.comment,   meta.comment);
        assert_eq!(restored.files.len(), meta.files.len());

        for (fi, (orig, rest)) in meta.files.iter().zip(restored.files.iter()).enumerate() {
            assert_eq!(orig.path,         rest.path,         "path fi={fi}");
            assert_eq!(orig.size,         rest.size,         "size fi={fi}");
            assert_eq!(orig.priority,     rest.priority,     "priority fi={fi}");
            assert_eq!(orig.piece_hashes, rest.piece_hashes, "piece_hashes fi={fi}");
        }
    }

    #[test]
    fn multifile_13_bitflow_file_written() {
        let meta = make_13_file_meta();
        let json = serde_json::to_string_pretty(&meta).expect("serializar");

        // Escribir el archivo .bitflow en el directorio de fixtures del crate.
        let out_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures");
        std::fs::create_dir_all(&out_path).expect("crear directorio fixtures");
        let file_path = out_path.join("multifile-13.bitflow");
        std::fs::write(&file_path, &json).expect("escribir .bitflow");

        // Verificar que el archivo existe y se puede releer.
        let raw = std::fs::read(&file_path).expect("leer .bitflow");
        let restored: Metainfo = serde_json::from_slice(&raw).expect("parsear .bitflow");
        assert_eq!(restored.files.len(), 13);
        assert_eq!(restored.info_hash, meta.info_hash);
    }
}
