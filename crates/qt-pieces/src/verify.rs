use sha2::{Digest, Sha256};

/// Verifica una pieza contra su hash SHA-256 esperado.
pub fn verify_piece(data: &[u8], expected_hash: &[u8; 32]) -> bool {
    let hash = Sha256::digest(data);
    hash.as_slice() == expected_hash
}

/// Calcula el hash SHA-256 de una pieza.
pub fn hash_piece(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_correct() {
        let data = b"hello piece";
        let hash = hash_piece(data);
        assert!(verify_piece(data, &hash));
    }

    #[test]
    fn verify_wrong() {
        let data = b"hello piece";
        assert!(!verify_piece(data, &[0u8; 32]));
    }

    #[test]
    fn hash_is_32_bytes() {
        let h = hash_piece(b"test");
        assert_eq!(h.len(), 32);
    }
}
