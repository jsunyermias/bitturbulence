use std::net::SocketAddr;
use bitturbulence_peer::PeerSession;
use bitturbulence_protocol::AuthPayload;
use bitturbulence_transport::QuicEndpoint;

const INFO_HASH: [u8; 32] = [0xAB; 32];
const PEER_ID_A: [u8; 32] = [0x41; 32];
const PEER_ID_B: [u8; 32] = [0x42; 32];

#[tokio::test]
async fn hello_succeeds() {
    let server_ep = QuicEndpoint::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap()).unwrap();
    let server_addr = server_ep.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<[u8; 32]>();

    tokio::spawn(async move {
        let conn = server_ep.accept().await.unwrap().unwrap();
        let session = PeerSession::new(conn, 1, PEER_ID_B, INFO_HASH, AuthPayload::None);
        let peer_id = session.hello_inbound().await.unwrap();
        tx.send(peer_id).unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    });

    let client_ep = QuicEndpoint::client_only().unwrap();
    let conn = client_ep.connect(server_addr).await.unwrap();
    let session = PeerSession::new(conn, 1, PEER_ID_A, INFO_HASH, AuthPayload::None);
    let peer_id = session.hello_outbound().await.unwrap();

    assert_eq!(peer_id, PEER_ID_B);
    assert_eq!(rx.await.unwrap(), PEER_ID_A);
}

#[tokio::test]
async fn wrong_info_hash_rejected() {
    let server_ep = QuicEndpoint::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap()).unwrap();
    let server_addr = server_ep.local_addr().unwrap();

    tokio::spawn(async move {
        let conn = server_ep.accept().await.unwrap().unwrap();
        let session = PeerSession::new(conn, 1, PEER_ID_B, INFO_HASH, AuthPayload::None);
        let _ = session.hello_inbound().await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    });

    let client_ep = QuicEndpoint::client_only().unwrap();
    let conn = client_ep.connect(server_addr).await.unwrap();
    let session = PeerSession::new(conn, 1, PEER_ID_A, [0xFF; 32], AuthPayload::None);
    assert!(session.hello_outbound().await.is_err());
}
