use crate::error::{ProtocolError, Result};

/// Prioridad de descarga de un archivo. 9 niveles.
/// Se transmite como u8 en el protocolo wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum Priority {
    Minimum  = 0,
    VeryLow  = 1,
    Lower    = 2,
    Low      = 3,
    Normal   = 4,
    High     = 5,
    Higher   = 6,
    VeryHigh = 7,
    Maximum  = 8,
}

impl Priority {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0 => Ok(Self::Minimum),
            1 => Ok(Self::VeryLow),
            2 => Ok(Self::Lower),
            3 => Ok(Self::Low),
            4 => Ok(Self::Normal),
            5 => Ok(Self::High),
            6 => Ok(Self::Higher),
            7 => Ok(Self::VeryHigh),
            8 => Ok(Self::Maximum),
            other => Err(ProtocolError::InvalidPriority(other)),
        }
    }

    pub fn as_u8(self) -> u8 { self as u8 }
}

impl Default for Priority {
    fn default() -> Self { Self::Normal }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        for v in 0u8..=8 {
            let p = Priority::from_u8(v).unwrap();
            assert_eq!(p.as_u8(), v);
        }
    }

    #[test]
    fn invalid_value_errors() {
        assert!(Priority::from_u8(9).is_err());
        assert!(Priority::from_u8(255).is_err());
    }

    #[test]
    fn ordering() {
        assert!(Priority::Maximum > Priority::Minimum);
        assert!(Priority::High > Priority::Normal);
        assert!(Priority::Normal > Priority::Low);
    }
}
