use std::time::Duration;
use futures_util::{SinkExt, StreamExt};
use tokio::time::timeout;
use tracing::{debug, info, warn};

use bitturbulence_protocol::{AuthPayload, Message, Priority, PROTOCOL_VERSION};
use bitturbulence_transport::PeerConnection;

use crate::{
    error::{PeerError, Result},
    state::{PendingRequest, SessionState},
};

const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const MSG_TIMEOUT:   Duration = Duration::from_secs(120);

pub trait SessionHandler: Send + 'static {
    fn on_piece(&mut self, file_index: u16, piece_index: u32, begin: u32, data: bytes::Bytes);
    fn on_have_piece(&mut self, file_index: u16, piece_index: u32);
    fn on_have_all(&mut self, file_index: u16);
    fn on_have_none(&mut self, file_index: u16);
    fn on_have_bitmap(&mut self, file_index: u16, bitmap: bytes::Bytes);
    fn on_priority_hint(&mut self, file_index: u16, priority: Priority);
    fn on_request(&mut self, file_index: u16, piece_index: u32, begin: u32, length: u32) -> Option<bytes::Bytes>;
}

pub struct PeerSession {
    conn: PeerConnection,
    pub state: SessionState,
    our_peer_id: [u8; 32],
    our_info_hash: [u8; 32],
    our_auth: AuthPayload,
}

impl PeerSession {
    pub fn new(
        conn: PeerConnection,
        num_files: usize,
        our_peer_id: [u8; 32],
        our_info_hash: [u8; 32],
        our_auth: AuthPayload,
    ) -> Self {
        Self {
            conn,
            state: SessionState::new(num_files),
            our_peer_id,
            our_info_hash,
            our_auth,
        }
    }

    /// Handshake como iniciador: enviamos Hello, esperamos HelloAck.
    pub async fn hello_outbound(&self) -> Result<[u8; 32]> {
        let (mut writer, mut reader) = self.conn.open_bidi_stream().await?;

        let hello = Message::Hello {
            version:   PROTOCOL_VERSION,
            peer_id:   self.our_peer_id,
            info_hash: self.our_info_hash,
            auth:      self.our_auth.clone(),
        };

        timeout(HELLO_TIMEOUT, writer.send(hello))
            .await.map_err(|_| PeerError::Timeout)??;

        let ack = timeout(HELLO_TIMEOUT, reader.next())
            .await.map_err(|_| PeerError::Timeout)?
            .ok_or(PeerError::Disconnected)??;

        match ack {
            Message::HelloAck { peer_id, accepted, reason } => {
                if !accepted {
                    return Err(PeerError::HelloRejected(
                        reason.unwrap_or_else(|| "no reason".into()),
                    ));
                }
                info!(peer = %self.conn.remote_addr(), "hello ok");
                Ok(peer_id)
            }
            _ => Err(PeerError::UnexpectedMessage),
        }
    }

    /// Handshake como receptor: esperamos Hello, respondemos HelloAck.
    pub async fn hello_inbound(&self) -> Result<[u8; 32]> {
        let (mut writer, mut reader) = self.conn.accept_bidi_stream().await?;

        let msg = timeout(HELLO_TIMEOUT, reader.next())
            .await.map_err(|_| PeerError::Timeout)?
            .ok_or(PeerError::Disconnected)??;

        match msg {
            Message::Hello { version: _, peer_id, info_hash, auth: _ } => {
                if info_hash != self.our_info_hash {
                    let ack = Message::HelloAck {
                        peer_id:  self.our_peer_id,
                        accepted: false,
                        reason:   Some("info hash mismatch".into()),
                    };
                    let _ = writer.send(ack).await;
                    return Err(PeerError::InfoHashMismatch);
                }

                let ack = Message::HelloAck {
                    peer_id:  self.our_peer_id,
                    accepted: true,
                    reason:   None,
                };
                timeout(HELLO_TIMEOUT, writer.send(ack))
                    .await.map_err(|_| PeerError::Timeout)??;

                info!(peer = %self.conn.remote_addr(), "hello accepted");
                Ok(peer_id)
            }
            _ => Err(PeerError::UnexpectedMessage),
        }
    }

    /// Loop principal de mensajes.
    pub async fn run<H: SessionHandler>(mut self, mut handler: H) -> Result<()> {
        let (mut writer, mut reader) = self.conn.open_bidi_stream().await?;

        loop {
            let msg = match timeout(MSG_TIMEOUT, reader.next()).await {
                Err(_)           => return Err(PeerError::Timeout),
                Ok(None)         => return Err(PeerError::Disconnected),
                Ok(Some(Err(e))) => return Err(PeerError::Protocol(e)),
                Ok(Some(Ok(m)))  => m,
            };

            debug!(peer = %self.conn.remote_addr(), ?msg, "received");

            match msg {
                Message::KeepAlive => {}

                Message::HaveAll { file_index } => {
                    self.state.apply_have_all(file_index);
                    handler.on_have_all(file_index);
                }

                Message::HaveNone { file_index } => {
                    self.state.apply_have_none(file_index);
                    handler.on_have_none(file_index);
                }

                Message::HavePiece { file_index, piece_index } => {
                    self.state.apply_have_piece(file_index, piece_index);
                    handler.on_have_piece(file_index, piece_index);
                }

                Message::HaveBitmap { file_index, bitmap } => {
                    self.state.apply_have_bitmap(file_index, &bitmap);
                    handler.on_have_bitmap(file_index, bitmap);
                }

                Message::Request { file_index, piece_index, begin, length } => {
                    if let Some(data) = handler.on_request(file_index, piece_index, begin, length) {
                        writer.send(Message::Piece { file_index, piece_index, begin, data }).await?;
                    } else {
                        writer.send(Message::Reject { file_index, piece_index, begin, length }).await?;
                    }
                }

                Message::Piece { file_index, piece_index, begin, data } => {
                    self.state.remove_pending(file_index, piece_index, begin);
                    handler.on_piece(file_index, piece_index, begin, data);
                }

                Message::Cancel { file_index, piece_index, begin, .. } => {
                    self.state.remove_pending(file_index, piece_index, begin);
                }

                Message::PriorityHint { file_index, priority } => {
                    handler.on_priority_hint(file_index, priority);
                }

                Message::Reject { .. } => {
                    warn!("request rejected by peer");
                }

                Message::Bye { reason } => {
                    info!(peer = %self.conn.remote_addr(), %reason, "peer said bye");
                    return Err(PeerError::Disconnected);
                }

                Message::Hello { .. } | Message::HelloAck { .. } => {
                    warn!("unexpected hello in session loop");
                }

                Message::HashRequest { .. } | Message::HashResponse { .. } => {
                    // Gestionados directamente en los data streams; ignorar aquí.
                }
            }
        }
    }

    pub async fn send_request(
        &mut self,
        file_index: u16,
        piece_index: u32,
        begin: u32,
        length: u32,
    ) -> Result<()> {
        self.state.add_pending(PendingRequest { file_index, piece_index, begin, length });
        self.conn.send_message(Message::Request { file_index, piece_index, begin, length }).await?;
        Ok(())
    }

    pub async fn send_bye(&self, reason: &str) -> Result<()> {
        self.conn.send_message(Message::Bye { reason: reason.into() }).await?;
        Ok(())
    }
}
