use std::path::{Path, PathBuf};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tracing::{debug, info};

use crate::error::{PiecesError, Result};

/// Almacén de piezas sobre un único fichero de datos.
/// En torrents multi-file, la capa superior concatena los ficheros
/// lógicamente y este store ve un único espacio de bytes contiguo.
pub struct PieceStore {
    path: PathBuf,
    piece_length: u64,
    total_length: u64,
    num_pieces: u32,
}

impl PieceStore {
    /// Crea o abre el fichero de almacenamiento.
    pub async fn open(
        path: impl AsRef<Path>,
        piece_length: u64,
        total_length: u64,
    ) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        // Crear el fichero si no existe, con el tamaño correcto.
        if !path.exists() {
            let file = File::create(&path).await?;
            file.set_len(total_length).await?;
            info!(path = %path.display(), total_length, "created piece store");
        }

        let num_pieces = ((total_length + piece_length - 1) / piece_length) as u32;

        Ok(Self { path, piece_length, total_length, num_pieces })
    }

    /// Lee un bloque de datos del disco.
    pub async fn read_block(&self, piece: u32, begin: u32, length: u32) -> Result<Vec<u8>> {
        self.check_bounds(piece, begin, length)?;

        let offset = piece as u64 * self.piece_length + begin as u64;
        let mut file = File::open(&self.path).await?;
        file.seek(std::io::SeekFrom::Start(offset)).await?;

        let mut buf = vec![0u8; length as usize];
        file.read_exact(&mut buf).await?;

        debug!(piece, begin, length, "read block");
        Ok(buf)
    }

    /// Escribe un bloque de datos en disco.
    pub async fn write_block(&self, piece: u32, begin: u32, data: &[u8]) -> Result<()> {
        self.check_bounds(piece, begin, data.len() as u32)?;

        let offset = piece as u64 * self.piece_length + begin as u64;
        let mut file = OpenOptions::new().write(true).open(&self.path).await?;
        file.seek(std::io::SeekFrom::Start(offset)).await?;
        file.write_all(data).await?;

        debug!(piece, begin, len = data.len(), "wrote block");
        Ok(())
    }

    /// Lee la pieza completa del disco.
    pub async fn read_piece(&self, piece: u32) -> Result<Vec<u8>> {
        if piece >= self.num_pieces {
            return Err(PiecesError::OutOfRange { index: piece, total: self.num_pieces });
        }
        let piece_len = self.piece_len(piece);
        self.read_block(piece, 0, piece_len as u32).await
    }

    pub fn num_pieces(&self) -> u32 {
        self.num_pieces
    }

    pub fn piece_length(&self) -> u64 {
        self.piece_length
    }

    pub fn total_length(&self) -> u64 {
        self.total_length
    }

    /// Longitud real de la pieza (la última puede ser más corta).
    pub fn piece_len(&self, piece: u32) -> u64 {
        let start = piece as u64 * self.piece_length;
        (self.total_length - start).min(self.piece_length)
    }

    fn check_bounds(&self, piece: u32, begin: u32, length: u32) -> Result<()> {
        if piece >= self.num_pieces {
            return Err(PiecesError::OutOfRange { index: piece, total: self.num_pieces });
        }
        let piece_len = self.piece_len(piece);
        if begin as u64 + length as u64 > piece_len {
            return Err(PiecesError::BlockOutOfBounds {
                piece, begin, length, piece_len,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn write_and_read_block() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.dat");

        let store = PieceStore::open(&path, 256, 1024).await.unwrap();

        let data = vec![0xAB_u8; 64];
        store.write_block(0, 0, &data).await.unwrap();

        let read = store.read_block(0, 0, 64).await.unwrap();
        assert_eq!(read, data);
    }

    #[tokio::test]
    async fn last_piece_shorter() {
        let store = PieceStore::open(
            tempfile::tempdir().unwrap().path().join("t.dat"),
            256, 300,
        ).await.unwrap();
        assert_eq!(store.num_pieces(), 2);
        assert_eq!(store.piece_len(0), 256);
        assert_eq!(store.piece_len(1), 44);
    }

    #[tokio::test]
    async fn out_of_range_error() {
        let dir = tempdir().unwrap();
        let store = PieceStore::open(dir.path().join("t.dat"), 256, 256).await.unwrap();
        assert!(store.read_block(1, 0, 10).await.is_err());
    }
}
