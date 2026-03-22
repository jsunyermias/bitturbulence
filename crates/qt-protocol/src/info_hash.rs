use sha2::{Digest, Sha256};
use std::fmt;
use crate::error::{ProtocolError, Result};

/// Info hash SHA-256 de 32 bytes. Identifica unívocamente un torrent.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct InfoHash([u8; 32]);

impl InfoHash {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 32 {
            return Err(ProtocolError::InvalidInfoHashLength(bytes.len()));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(bytes);
        Ok(Self(arr))
    }

    /// Calcula el hash SHA-256 del contenido del dict `info` serializado.
    pub fn from_info_bytes(info_bytes: &[u8]) -> Self {
        let hash = Sha256::digest(info_bytes);
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&hash);
        Self(arr)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for InfoHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InfoHash({})", self)
    }
}

impl fmt::Display for InfoHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in &self.0 { write!(f, "{:02x}", b)?; }
        Ok(())
    }
}

impl TryFrom<&[u8]> for InfoHash {
    type Error = ProtocolError;
    fn try_from(bytes: &[u8]) -> Result<Self> { Self::from_bytes(bytes) }
}

impl From<[u8; 32]> for InfoHash {
    fn from(arr: [u8; 32]) -> Self { Self(arr) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_64_hex_chars() {
        let h = InfoHash::from_info_bytes(b"hello");
        assert_eq!(h.to_string().len(), 64);
    }

    #[test]
    fn wrong_length_errors() {
        assert!(InfoHash::from_bytes(&[0u8; 31]).is_err());
        assert!(InfoHash::from_bytes(&[0u8; 33]).is_err());
    }

    #[test]
    fn correct_length_ok() {
        assert!(InfoHash::from_bytes(&[0u8; 32]).is_ok());
    }
}
