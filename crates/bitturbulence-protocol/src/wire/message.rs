use bytes::{Buf, BufMut, Bytes, BytesMut};
use crate::{
    auth::AuthPayload,
    error::{ProtocolError, Result},
    priority::Priority,
};

// IDs de mensaje
const MSG_KEEPALIVE:     u8 = 0;
const MSG_HELLO:         u8 = 1;
const MSG_HELLO_ACK:     u8 = 2;
const MSG_HAVE_ALL:      u8 = 3;
const MSG_HAVE_NONE:     u8 = 4;
const MSG_HAVE_PIECE:    u8 = 5;
const MSG_HAVE_BITMAP:   u8 = 6;
const MSG_REQUEST:       u8 = 7;
const MSG_PIECE:         u8 = 8;
const MSG_CANCEL:        u8 = 9;
const MSG_REJECT:        u8 = 10;
const MSG_PRIORITY_HINT: u8 = 11;
const MSG_HASH_REQUEST:  u8 = 12;
const MSG_HASH_RESPONSE: u8 = 13;
const MSG_BYE:           u8 = 14;

/// Versión actual del protocolo BitTurbulence.
pub const PROTOCOL_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    KeepAlive,

    /// Primer mensaje de la sesión — identifica al peer y presenta credenciales.
    Hello {
        version: u8,
        peer_id: [u8; 32],
        info_hash: [u8; 32],
        auth: AuthPayload,
    },

    /// Respuesta al Hello.
    HelloAck {
        peer_id: [u8; 32],
        accepted: bool,
        reason: Option<String>,
    },

    /// Tengo el archivo completo.
    HaveAll { file_index: u16 },

    /// No tengo nada de este archivo.
    HaveNone { file_index: u16 },

    /// Tengo esta pieza concreta.
    HavePiece { file_index: u16, piece_index: u32 },

    /// Bitmap compacto de piezas disponibles para un archivo.
    HaveBitmap { file_index: u16, bitmap: Bytes },

    /// Solicitar un bloque de datos.
    Request { file_index: u16, piece_index: u32, begin: u32, length: u32 },

    /// Datos de un bloque.
    Piece { file_index: u16, piece_index: u32, begin: u32, data: Bytes },

    /// Cancelar una request pendiente.
    Cancel { file_index: u16, piece_index: u32, begin: u32, length: u32 },

    /// No puedo servir esta request.
    Reject { file_index: u16, piece_index: u32, begin: u32, length: u32 },

    /// Sugerencia de prioridad para un archivo.
    PriorityHint { file_index: u16, priority: Priority },

    /// Solicitar los hashes de bloque de una pieza para verificación incremental.
    /// El receptor responde con HashResponse. Si no tiene la pieza, responde
    /// con block_hashes vacío.
    HashRequest { file_index: u16, piece_index: u32 },

    /// Respuesta con los hashes SHA-256 de cada bloque de una pieza.
    /// `block_hashes[i] = SHA-256(bloque_i)`. Vacío si el peer no tiene la pieza.
    /// La raíz Merkle de estos hashes debe coincidir con piece_hashes[pi] del metainfo.
    HashResponse { file_index: u16, piece_index: u32, block_hashes: Vec<[u8; 32]> },

    /// Cierre limpio de la sesión.
    Bye { reason: String },
}

impl Message {
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::new();
        match self {
            Message::KeepAlive => {}

            Message::Hello { version, peer_id, info_hash, auth } => {
                buf.put_u8(MSG_HELLO);
                buf.put_u8(*version);
                buf.put_slice(peer_id);
                buf.put_slice(info_hash);
                let auth_bytes = auth.encode();
                buf.put_u16(auth_bytes.len() as u16);
                buf.put_slice(&auth_bytes);
            }

            Message::HelloAck { peer_id, accepted, reason } => {
                buf.put_u8(MSG_HELLO_ACK);
                buf.put_slice(peer_id);
                buf.put_u8(if *accepted { 1 } else { 0 });
                match reason {
                    None => buf.put_u16(0),
                    Some(r) => {
                        let rb = r.as_bytes();
                        buf.put_u16(rb.len() as u16);
                        buf.put_slice(rb);
                    }
                }
            }

            Message::HaveAll  { file_index } => { buf.put_u8(MSG_HAVE_ALL);  buf.put_u16(*file_index); }
            Message::HaveNone { file_index } => { buf.put_u8(MSG_HAVE_NONE); buf.put_u16(*file_index); }

            Message::HavePiece { file_index, piece_index } => {
                buf.put_u8(MSG_HAVE_PIECE);
                buf.put_u16(*file_index);
                buf.put_u32(*piece_index);
            }

            Message::HaveBitmap { file_index, bitmap } => {
                buf.put_u8(MSG_HAVE_BITMAP);
                buf.put_u16(*file_index);
                buf.put_u32(bitmap.len() as u32);
                buf.put_slice(bitmap);
            }

            Message::Request { file_index, piece_index, begin, length } => {
                buf.put_u8(MSG_REQUEST);
                buf.put_u16(*file_index);
                buf.put_u32(*piece_index);
                buf.put_u32(*begin);
                buf.put_u32(*length);
            }

            Message::Piece { file_index, piece_index, begin, data } => {
                buf.put_u8(MSG_PIECE);
                buf.put_u16(*file_index);
                buf.put_u32(*piece_index);
                buf.put_u32(*begin);
                buf.put_slice(data);
            }

            Message::Cancel { file_index, piece_index, begin, length } => {
                buf.put_u8(MSG_CANCEL);
                buf.put_u16(*file_index);
                buf.put_u32(*piece_index);
                buf.put_u32(*begin);
                buf.put_u32(*length);
            }

            Message::Reject { file_index, piece_index, begin, length } => {
                buf.put_u8(MSG_REJECT);
                buf.put_u16(*file_index);
                buf.put_u32(*piece_index);
                buf.put_u32(*begin);
                buf.put_u32(*length);
            }

            Message::PriorityHint { file_index, priority } => {
                buf.put_u8(MSG_PRIORITY_HINT);
                buf.put_u16(*file_index);
                buf.put_u8(priority.as_u8());
            }

            Message::HashRequest { file_index, piece_index } => {
                buf.put_u8(MSG_HASH_REQUEST);
                buf.put_u16(*file_index);
                buf.put_u32(*piece_index);
            }

            Message::HashResponse { file_index, piece_index, block_hashes } => {
                buf.put_u8(MSG_HASH_RESPONSE);
                buf.put_u16(*file_index);
                buf.put_u32(*piece_index);
                buf.put_u32(block_hashes.len() as u32);
                for h in block_hashes {
                    buf.put_slice(h.as_slice());
                }
            }

            Message::Bye { reason } => {
                buf.put_u8(MSG_BYE);
                let rb = reason.as_bytes();
                buf.put_u16(rb.len() as u16);
                buf.put_slice(rb);
            }
        }
        buf.freeze()
    }

    pub fn decode(mut buf: Bytes) -> Result<Self> {
        if buf.is_empty() { return Ok(Message::KeepAlive); }
        let id = buf.get_u8();
        match id {
            MSG_KEEPALIVE => Ok(Message::KeepAlive),

            MSG_HELLO => {
                chk(&buf, 1 + 32 + 32 + 2, "Hello")?;
                let version   = buf.get_u8();
                let peer_id   = r32(&mut buf);
                let info_hash = r32(&mut buf);
                let auth_len  = buf.get_u16() as usize;
                chk(&buf, auth_len, "Hello.auth")?;
                let mut auth_bytes = buf.copy_to_bytes(auth_len);
                let auth = AuthPayload::decode(&mut auth_bytes)?;
                Ok(Message::Hello { version, peer_id, info_hash, auth })
            }

            MSG_HELLO_ACK => {
                chk(&buf, 32 + 1 + 2, "HelloAck")?;
                let peer_id  = r32(&mut buf);
                let accepted = buf.get_u8() != 0;
                let reason_len = buf.get_u16() as usize;
                let reason = if reason_len == 0 {
                    None
                } else {
                    chk(&buf, reason_len, "HelloAck.reason")?;
                    Some(String::from_utf8(buf.copy_to_bytes(reason_len).to_vec())?)
                };
                Ok(Message::HelloAck { peer_id, accepted, reason })
            }

            MSG_HAVE_ALL  => { chk(&buf, 2, "HaveAll")?;  Ok(Message::HaveAll  { file_index: buf.get_u16() }) }
            MSG_HAVE_NONE => { chk(&buf, 2, "HaveNone")?; Ok(Message::HaveNone { file_index: buf.get_u16() }) }

            MSG_HAVE_PIECE => {
                chk(&buf, 6, "HavePiece")?;
                Ok(Message::HavePiece { file_index: buf.get_u16(), piece_index: buf.get_u32() })
            }

            MSG_HAVE_BITMAP => {
                chk(&buf, 6, "HaveBitmap")?;
                let file_index = buf.get_u16();
                let len = buf.get_u32() as usize;
                chk(&buf, len, "HaveBitmap.bitmap")?;
                Ok(Message::HaveBitmap { file_index, bitmap: buf.copy_to_bytes(len) })
            }

            MSG_REQUEST => {
                chk(&buf, 14, "Request")?;
                Ok(Message::Request {
                    file_index:  buf.get_u16(),
                    piece_index: buf.get_u32(),
                    begin:       buf.get_u32(),
                    length:      buf.get_u32(),
                })
            }

            MSG_PIECE => {
                chk(&buf, 10, "Piece")?;
                let file_index  = buf.get_u16();
                let piece_index = buf.get_u32();
                let begin       = buf.get_u32();
                Ok(Message::Piece { file_index, piece_index, begin, data: buf })
            }

            MSG_CANCEL => {
                chk(&buf, 14, "Cancel")?;
                Ok(Message::Cancel {
                    file_index:  buf.get_u16(),
                    piece_index: buf.get_u32(),
                    begin:       buf.get_u32(),
                    length:      buf.get_u32(),
                })
            }

            MSG_REJECT => {
                chk(&buf, 14, "Reject")?;
                Ok(Message::Reject {
                    file_index:  buf.get_u16(),
                    piece_index: buf.get_u32(),
                    begin:       buf.get_u32(),
                    length:      buf.get_u32(),
                })
            }

            MSG_PRIORITY_HINT => {
                chk(&buf, 3, "PriorityHint")?;
                let file_index = buf.get_u16();
                let priority   = Priority::from_u8(buf.get_u8())?;
                Ok(Message::PriorityHint { file_index, priority })
            }

            MSG_HASH_REQUEST => {
                chk(&buf, 6, "HashRequest")?;
                Ok(Message::HashRequest {
                    file_index:  buf.get_u16(),
                    piece_index: buf.get_u32(),
                })
            }

            MSG_HASH_RESPONSE => {
                chk(&buf, 10, "HashResponse")?;
                let file_index  = buf.get_u16();
                let piece_index = buf.get_u32();
                let count       = buf.get_u32() as usize;
                chk(&buf, count * 32, "HashResponse.hashes")?;
                let mut block_hashes = Vec::with_capacity(count);
                for _ in 0..count {
                    block_hashes.push(r32(&mut buf));
                }
                Ok(Message::HashResponse { file_index, piece_index, block_hashes })
            }

            MSG_BYE => {
                chk(&buf, 2, "Bye")?;
                let len = buf.get_u16() as usize;
                chk(&buf, len, "Bye.reason")?;
                let reason = String::from_utf8(buf.copy_to_bytes(len).to_vec())?;
                Ok(Message::Bye { reason })
            }

            other => Err(ProtocolError::UnknownMessageId(other)),
        }
    }
}

fn chk(buf: &Bytes, n: usize, _ctx: &str) -> Result<()> {
    if buf.remaining() < n {
        Err(ProtocolError::InvalidMessageLength { expected: n, got: buf.remaining() })
    } else {
        Ok(())
    }
}

fn r32(buf: &mut Bytes) -> [u8; 32] {
    let mut a = [0u8; 32];
    buf.copy_to_slice(&mut a);
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthPayload;

    fn rt(msg: Message) {
        let encoded = msg.encode();
        let decoded = Message::decode(encoded).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test] fn keepalive()    { rt(Message::KeepAlive); }
    #[test] fn have_all()     { rt(Message::HaveAll  { file_index: 3 }); }
    #[test] fn have_none()    { rt(Message::HaveNone { file_index: 0 }); }
    #[test] fn have_piece()   { rt(Message::HavePiece { file_index: 1, piece_index: 42 }); }
    #[test] fn have_bitmap()  { rt(Message::HaveBitmap { file_index: 0, bitmap: Bytes::from(vec![0b1010_1010]) }); }
    #[test] fn request()      { rt(Message::Request { file_index: 0, piece_index: 5, begin: 0, length: 16384 }); }
    #[test] fn piece()        { rt(Message::Piece { file_index: 0, piece_index: 5, begin: 0, data: Bytes::from_static(b"data") }); }
    #[test] fn cancel()       { rt(Message::Cancel { file_index: 0, piece_index: 5, begin: 0, length: 16384 }); }
    #[test] fn reject()       { rt(Message::Reject { file_index: 0, piece_index: 5, begin: 0, length: 16384 }); }
    #[test] fn priority_hint(){ rt(Message::PriorityHint { file_index: 2, priority: Priority::High }); }
    #[test] fn bye()          { rt(Message::Bye { reason: "done".into() }); }

    #[test]
    fn hello_with_credentials() {
        rt(Message::Hello {
            version:   PROTOCOL_VERSION,
            peer_id:   [0x01; 32],
            info_hash: [0x02; 32],
            auth: AuthPayload::Credentials {
                user: "jordi".into(),
                password_hash: [0x42; 32],
            },
        });
    }

    #[test]
    fn hello_ack_with_reason() {
        rt(Message::HelloAck {
            peer_id:  [0x02; 32],
            accepted: false,
            reason:   Some("invalid credentials".into()),
        });
    }

    #[test]
    fn hash_request() {
        rt(Message::HashRequest { file_index: 2, piece_index: 77 });
    }

    #[test]
    fn hash_response_empty() {
        rt(Message::HashResponse { file_index: 0, piece_index: 1, block_hashes: vec![] });
    }

    #[test]
    fn hash_response_multiple_hashes() {
        let h0 = [0x11u8; 32];
        let h1 = [0x22u8; 32];
        let h2 = [0x33u8; 32];
        rt(Message::HashResponse {
            file_index: 1,
            piece_index: 42,
            block_hashes: vec![h0, h1, h2],
        });
    }

    #[test]
    fn unknown_id_errors() {
        assert!(Message::decode(Bytes::from(vec![0xFFu8])).is_err());
    }
}
