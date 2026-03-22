use std::path::Path;
use qt_protocol::Metainfo;

use crate::{error::Result, store::PieceStore};

/// Gestiona el almacenamiento completo de un torrent.
///
/// Mantiene un [`PieceStore`] por cada archivo en el [`Metainfo`], enrutando
/// las operaciones de lectura y escritura por `file_index`. Esto materializa
/// la capa superior que `PieceStore` deja a cargo del llamador.
pub struct TorrentStore {
    stores: Vec<PieceStore>,
}

impl TorrentStore {
    /// Crea o abre los ficheros de datos para todos los archivos del torrent.
    ///
    /// Los ficheros se ubican en `base_dir/<metainfo.name>/<file.path>`.
    /// Los directorios intermedios se crean automáticamente.
    pub async fn open(base_dir: &Path, meta: &Metainfo) -> Result<Self> {
        let mut stores = Vec::with_capacity(meta.files.len());
        for file in &meta.files {
            let mut path = base_dir.join(&meta.name);
            for component in &file.path {
                path = path.join(component);
            }
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let store = PieceStore::open(&path, file.piece_length() as u64, file.size).await?;
            stores.push(store);
        }
        Ok(Self { stores })
    }

    /// Número de archivos gestionados.
    pub fn num_files(&self) -> usize {
        self.stores.len()
    }

    /// Accede al store de un archivo concreto.
    pub fn file(&self, file_index: usize) -> &PieceStore {
        &self.stores[file_index]
    }

    /// Lee un bloque de datos de un archivo.
    pub async fn read_block(
        &self,
        file_index: usize,
        piece: u32,
        begin: u32,
        length: u32,
    ) -> Result<Vec<u8>> {
        self.stores[file_index].read_block(piece, begin, length).await
    }

    /// Escribe un bloque de datos en un archivo.
    pub async fn write_block(
        &self,
        file_index: usize,
        piece: u32,
        begin: u32,
        data: &[u8],
    ) -> Result<()> {
        self.stores[file_index].write_block(piece, begin, data).await
    }

    /// Lee una pieza completa de un archivo y la devuelve para verificación.
    pub async fn read_piece(&self, file_index: usize, piece: u32) -> Result<Vec<u8>> {
        self.stores[file_index].read_piece(piece).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qt_protocol::{FileEntry, Metainfo, Priority};
    use tempfile::tempdir;

    fn make_meta(files: &[(&str, u64)]) -> Metainfo {
        Metainfo {
            name: "test-torrent".into(),
            info_hash: [0u8; 32],
            files: files.iter().map(|(name, size)| {
                FileEntry::new(vec![name.to_string()], *size, Priority::Normal)
            }).collect(),
            trackers: vec![],
            comment: None,
        }
    }

    #[tokio::test]
    async fn open_creates_files() {
        let dir = tempdir().unwrap();
        let meta = make_meta(&[("a.bin", 1024), ("b.bin", 2048)]);
        let store = TorrentStore::open(dir.path(), &meta).await.unwrap();
        assert_eq!(store.num_files(), 2);
        assert!(dir.path().join("test-torrent/a.bin").exists());
        assert!(dir.path().join("test-torrent/b.bin").exists());
    }

    #[tokio::test]
    async fn write_and_read_block_by_file_index() {
        let dir = tempdir().unwrap();
        let meta = make_meta(&[("x.bin", 4096), ("y.bin", 4096)]);
        let store = TorrentStore::open(dir.path(), &meta).await.unwrap();

        let data = vec![0xBE_u8; 128];
        store.write_block(1, 0, 0, &data).await.unwrap();

        let read = store.read_block(1, 0, 0, 128).await.unwrap();
        assert_eq!(read, data);

        // El archivo 0 no fue tocado
        let untouched = store.read_block(0, 0, 0, 128).await.unwrap();
        assert_eq!(untouched, vec![0u8; 128]);
    }
}
