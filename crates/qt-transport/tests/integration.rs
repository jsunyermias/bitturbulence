use std::net::SocketAddr;
use futures_util::{SinkExt, StreamExt};
use qt_protocol::Message;
use qt_transport::QuicEndpoint;

#[tokio::test]
async fn two_peers_exchange_message() {
    let server_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let server = QuicEndpoint::bind(server_addr).unwrap();
    let server_addr = server.local_addr().unwrap();

    let server_task = tokio::spawn(async move {
        let conn = server.accept().await.unwrap().unwrap();
        let (_writer, mut reader) = conn.accept_bidi_stream().await.unwrap();
        reader.next().await.unwrap().unwrap()
    });

    let client = QuicEndpoint::client_only().unwrap();
    let peer = client.connect(server_addr).await.unwrap();
    let (mut writer, _reader) = peer.open_bidi_stream().await.unwrap();
    writer.send(Message::HavePiece { file_index: 0, piece_index: 42 }).await.unwrap();
    writer.close().await.unwrap();

    let received = server_task.await.unwrap();
    assert_eq!(received, Message::HavePiece { file_index: 0, piece_index: 42 });
}
