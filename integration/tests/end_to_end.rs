use std::net::SocketAddr;
use bytes::Bytes;
use tempfile::tempdir;

use bitturbulence_pieces::{hash_piece, verify_piece, PieceStore};
use bitturbulence_protocol::{AuthPayload, Message, MessageCodec, PROTOCOL_VERSION};
use bitturbulence_transport::QuicEndpoint;
use tokio_util::codec::{FramedRead, FramedWrite};
use futures_util::{SinkExt, StreamExt};

const PIECE_LEN: u64 = 256;
const TOTAL_LEN: u64 = 256;
const BLOCK_LEN: u32 = 256;
const INFO_HASH: [u8; 32] = [0xAB; 32];
const PEER_ID_A: [u8; 32] = [0x41; 32];
const PEER_ID_B: [u8; 32] = [0x42; 32];

fn make_piece() -> (Vec<u8>, [u8; 32]) {
    let data: Vec<u8> = (0..PIECE_LEN as usize).map(|i| (i % 251) as u8).collect();
    let hash = hash_piece(&data);
    (data, hash)
}

#[tokio::test(flavor = "multi_thread")]
async fn full_download_one_piece() {
    let (piece_data, piece_hash) = make_piece();

    let server_dir = tempdir().unwrap();
    let server_store = PieceStore::open(
        server_dir.path().join("server.bin"), PIECE_LEN, TOTAL_LEN,
    ).await.unwrap();
    server_store.write_block(0, 0, &piece_data).await.unwrap();

    let client_dir = tempdir().unwrap();
    let client_store = PieceStore::open(
        client_dir.path().join("client.bin"), PIECE_LEN, TOTAL_LEN,
    ).await.unwrap();

    let server_ep = QuicEndpoint::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap()).unwrap();
    let server_addr = server_ep.local_addr().unwrap();
    let client_ep = QuicEndpoint::client_only().unwrap();

    let (result_tx, result_rx) = tokio::sync::oneshot::channel::<bool>();

    // ── Task servidor ─────────────────────────────────────────────────────────
    tokio::spawn(async move {
        let server_conn = server_ep.accept().await.unwrap().unwrap();
        let (send, recv) = server_conn.inner_conn().accept_bi().await.unwrap();
        let mut writer = FramedWrite::new(send, MessageCodec);
        let mut reader = FramedRead::new(recv, MessageCodec);

        // Ignorar KeepAlive inicial, esperar Hello
        loop {
            match reader.next().await.unwrap().unwrap() {
                Message::KeepAlive => continue,
                Message::Hello { info_hash, .. } => {
                    assert_eq!(info_hash, INFO_HASH);
                    break;
                }
                m => panic!("expected Hello, got {:?}", m),
            }
        };

        writer.send(Message::HelloAck {
            peer_id: PEER_ID_B,
            accepted: true,
            reason: None,
        }).await.unwrap();

        // Anunciar que tenemos el archivo 0 completo
        writer.send(Message::HaveAll { file_index: 0 }).await.unwrap();

        // Esperar Request
        let (fi, pi, begin, length) = loop {
            match reader.next().await.unwrap().unwrap() {
                Message::KeepAlive => continue,
                Message::Request { file_index, piece_index, begin, length } => {
                    break (file_index, piece_index, begin, length);
                }
                m => panic!("expected Request, got {:?}", m),
            }
        };

        let data = server_store.read_block(pi, begin, length).await.unwrap();
        writer.send(Message::Piece {
            file_index: fi, piece_index: pi, begin,
            data: Bytes::from(data),
        }).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // ── Task cliente ──────────────────────────────────────────────────────────
    tokio::spawn(async move {
        let client_conn = client_ep.connect(server_addr).await.unwrap();
        let (send, recv) = client_conn.inner_conn().open_bi().await.unwrap();
        let mut writer = FramedWrite::new(send, MessageCodec);
        let mut reader = FramedRead::new(recv, MessageCodec);

        // Materializar stream
        writer.send(Message::KeepAlive).await.unwrap();

        // Enviar Hello
        writer.send(Message::Hello {
            version: PROTOCOL_VERSION,
            peer_id: PEER_ID_A,
            info_hash: INFO_HASH,
            auth: AuthPayload::None,
        }).await.unwrap();

        // Esperar HelloAck
        loop {
            match reader.next().await.unwrap().unwrap() {
                Message::KeepAlive => continue,
                Message::HelloAck { accepted, .. } => {
                    assert!(accepted);
                    break;
                }
                m => panic!("expected HelloAck, got {:?}", m),
            }
        }

        // Esperar HaveAll
        loop {
            match reader.next().await.unwrap().unwrap() {
                Message::KeepAlive => continue,
                Message::HaveAll { .. } => break,
                m => panic!("expected HaveAll, got {:?}", m),
            }
        }

        // Enviar Request
        writer.send(Message::Request {
            file_index: 0, piece_index: 0, begin: 0, length: BLOCK_LEN,
        }).await.unwrap();

        // Esperar Piece
        let (piece_index, begin, data) = loop {
            match reader.next().await.unwrap().unwrap() {
                Message::KeepAlive => continue,
                Message::Piece { piece_index, begin, data, .. } => break (piece_index, begin, data),
                m => panic!("expected Piece, got {:?}", m),
            }
        };

        client_store.write_block(piece_index, begin, &data).await.unwrap();
        let stored = client_store.read_piece(piece_index).await.unwrap();
        result_tx.send(verify_piece(&stored, &piece_hash)).unwrap();
    });

    let verified = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        result_rx,
    ).await.expect("test timed out").unwrap();

    assert!(verified, "piece SHA-256 verification failed");
}
