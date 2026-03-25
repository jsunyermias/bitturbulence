//! # bitturbulence-protocol

//!
//! Tipos base del protocolo BitTurbulence: mensajes wire, metainfo,
//! info hash SHA-256, prioridades y autenticación.
//!
//! ## Módulos principales
//!
//! - [`wire`] — mensajes del protocolo y codec length-prefixed
//! - [`metainfo`] — estructura del torrent y cálculo de piece_length adaptativo
//! - [`auth`] — payload de autenticación para el mensaje Hello
//! - [`priority`] — 9 niveles de prioridad de descarga
//! - [`info_hash`] — info hash SHA-256 de 32 bytes

pub mod auth;
pub mod error;
pub mod info_hash;
pub mod metainfo;
pub mod priority;
pub mod wire;

pub use auth::AuthPayload;
pub use error::{ProtocolError, Result};
pub use info_hash::InfoHash;
pub use metainfo::{FileEntry, Metainfo, num_pieces, piece_length_for_size, BLOCK_SIZE};
pub use priority::Priority;
pub use wire::{Message, MessageCodec, PROTOCOL_VERSION};
