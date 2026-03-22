use bytes::{Buf, BufMut, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use crate::error::ProtocolError;
use super::message::Message;

/// Tamaño máximo de frame: 256 MiB + overhead.
/// El tamaño máximo de pieza es 256 MB, más cabecera de mensaje.
pub const MAX_FRAME_SIZE: usize = 256 * 1024 * 1024 + 64;

pub struct MessageCodec;

impl Decoder for MessageCodec {
    type Item = Message;
    type Error = ProtocolError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 4 { src.reserve(4); return Ok(None); }
        let length = u32::from_be_bytes([src[0], src[1], src[2], src[3]]) as usize;
        if length == 0 { src.advance(4); return Ok(Some(Message::KeepAlive)); }
        if length > MAX_FRAME_SIZE { return Err(ProtocolError::FrameTooLarge(length)); }
        if src.len() < 4 + length { src.reserve(4 + length - src.len()); return Ok(None); }
        src.advance(4);
        let payload = src.split_to(length).freeze();
        Ok(Some(Message::decode(payload)?))
    }
}

impl Encoder<Message> for MessageCodec {
    type Error = ProtocolError;

    fn encode(&mut self, item: Message, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let payload = item.encode();
        if payload.len() > MAX_FRAME_SIZE { return Err(ProtocolError::FrameTooLarge(payload.len())); }
        dst.reserve(4 + payload.len());
        dst.put_u32(payload.len() as u32);
        dst.put_slice(&payload);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::codec::{Decoder, Encoder};

    fn rt(msg: Message) -> Message {
        let mut codec = MessageCodec;
        let mut buf = BytesMut::new();
        codec.encode(msg, &mut buf).unwrap();
        codec.decode(&mut buf).unwrap().unwrap()
    }

    #[test] fn keepalive()  { assert_eq!(rt(Message::KeepAlive), Message::KeepAlive); }
    #[test] fn have_piece() { assert_eq!(rt(Message::HavePiece { file_index: 0, piece_index: 7 }), Message::HavePiece { file_index: 0, piece_index: 7 }); }
    #[test] fn partial()    { let mut c = MessageCodec; let mut b = BytesMut::from(&b"\x00\x00\x00"[..]); assert!(c.decode(&mut b).unwrap().is_none()); }
    #[test] fn too_large()  { let mut c = MessageCodec; let mut b = BytesMut::new(); b.put_u32((MAX_FRAME_SIZE + 1) as u32); assert!(c.decode(&mut b).is_err()); }
}
