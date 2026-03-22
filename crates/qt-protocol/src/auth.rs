use bytes::{Buf, BufMut, Bytes, BytesMut};
use crate::error::{ProtocolError, Result};

const TAG_NONE:        u8 = 0;
const TAG_CREDENTIALS: u8 = 1;
const TAG_TOKEN:       u8 = 2;
const TAG_CERTIFICATE: u8 = 3;

/// Payload de autenticación incluido en el mensaje Hello.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthPayload {
    /// Sin autenticación — modo público.
    None,
    /// Usuario + SHA-256 de la contraseña.
    Credentials { user: String, password_hash: [u8; 32] },
    /// API key / token de 32 bytes.
    Token([u8; 32]),
    /// mTLS — el certificado cliente ya fue validado en el handshake TLS.
    /// Aquí solo señalizamos que queremos autenticación por certificado.
    Certificate,
}

impl AuthPayload {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::new();
        match self {
            AuthPayload::None => {
                buf.put_u8(TAG_NONE);
            }
            AuthPayload::Credentials { user, password_hash } => {
                buf.put_u8(TAG_CREDENTIALS);
                let user_bytes = user.as_bytes();
                buf.put_u16(user_bytes.len() as u16);
                buf.put_slice(user_bytes);
                buf.put_slice(password_hash);
            }
            AuthPayload::Token(token) => {
                buf.put_u8(TAG_TOKEN);
                buf.put_slice(token);
            }
            AuthPayload::Certificate => {
                buf.put_u8(TAG_CERTIFICATE);
            }
        }
        buf.freeze()
    }

    pub fn decode(buf: &mut Bytes) -> Result<Self> {
        if buf.is_empty() {
            return Err(ProtocolError::InvalidMessageLength { expected: 1, got: 0 });
        }
        let tag = buf.get_u8();
        match tag {
            TAG_NONE => Ok(AuthPayload::None),
            TAG_CREDENTIALS => {
                if buf.remaining() < 2 {
                    return Err(ProtocolError::InvalidMessageLength { expected: 2, got: buf.remaining() });
                }
                let user_len = buf.get_u16() as usize;
                if buf.remaining() < user_len + 32 {
                    return Err(ProtocolError::InvalidMessageLength {
                        expected: user_len + 32,
                        got: buf.remaining(),
                    });
                }
                let user = String::from_utf8(buf.copy_to_bytes(user_len).to_vec())?;
                let mut hash = [0u8; 32];
                buf.copy_to_slice(&mut hash);
                Ok(AuthPayload::Credentials { user, password_hash: hash })
            }
            TAG_TOKEN => {
                if buf.remaining() < 32 {
                    return Err(ProtocolError::InvalidMessageLength { expected: 32, got: buf.remaining() });
                }
                let mut token = [0u8; 32];
                buf.copy_to_slice(&mut token);
                Ok(AuthPayload::Token(token))
            }
            TAG_CERTIFICATE => Ok(AuthPayload::Certificate),
            other => Err(ProtocolError::InvalidAuthTag(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(auth: AuthPayload) {
        let encoded = auth.encode();
        let mut buf = encoded;
        let decoded = AuthPayload::decode(&mut buf).unwrap();
        assert_eq!(auth, decoded); // necesita PartialEq
    }

    #[test] fn none()        { rt(AuthPayload::None); }
    #[test] fn certificate() { rt(AuthPayload::Certificate); }
    #[test] fn token()       { rt(AuthPayload::Token([0xAB; 32])); }
    #[test] fn credentials() {
        rt(AuthPayload::Credentials {
            user: "jordi".into(),
            password_hash: [0x42; 32],
        });
    }
}
