//! Descargador mínimo: descarga un archivo de un seeder.
//! Uso: cargo run --bin download -- <ip:puerto> <info_hash_hex> <num_pieces> <piece_length> <output>

use std::io::SeekFrom;
use std::net::SocketAddr;
use futures_util::{SinkExt, StreamExt};
use qt_pieces::hash_piece;
use qt_protocol::{AuthPayload, Message, MessageCodec, PROTOCOL_VERSION};
use qt_transport::QuicEndpoint;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio_util::codec::{FramedRead, FramedWrite};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 6 {
        eprintln!("uso: download <ip:puerto> <info_hash_hex> <num_pieces> <piece_length> <output>");
        std::process::exit(1);
    }

    let addr: SocketAddr    = args[1].parse()?;
    let info_hash_hex       = &args[2];
    let num_pieces: usize   = args[3].parse()?;
    let piece_length: usize = args[4].parse()?;
    let output_path         = &args[5];

    let info_hash_bytes = hex::decode(info_hash_hex)?;
    let mut info_hash = [0u8; 32];
    info_hash.copy_from_slice(&info_hash_bytes);

    println!("conectando a {} ...", addr);

    let endpoint = QuicEndpoint::client_only()?;
    let conn = endpoint.connect(addr).await?;

    let (send, recv) = conn.inner_conn().open_bi().await?;
    let mut writer = FramedWrite::new(send, MessageCodec);
    let mut reader = FramedRead::new(recv, MessageCodec);

    // Materializar stream
    writer.send(Message::KeepAlive).await?;

    // Hello
    writer.send(Message::Hello {
        version:   PROTOCOL_VERSION,
        peer_id:   [0x44u8; 32], // 'D' de Downloader
        info_hash,
        auth:      AuthPayload::None,
    }).await?;

    // Esperar HelloAck
    loop {
        match reader.next().await.unwrap()? {
            Message::KeepAlive => continue,
            Message::HelloAck { accepted, reason, .. } => {
                if !accepted {
                    anyhow::bail!("rechazado: {}", reason.unwrap_or_default());
                }
                println!("hello aceptado");
                break;
            }
            m => println!("ignorando: {:?}", m),
        }
    }

    // Esperar HaveAll
    loop {
        match reader.next().await.unwrap()? {
            Message::KeepAlive => continue,
            Message::HaveAll { .. } => { println!("seeder tiene todo"); break; }
            m => println!("ignorando: {:?}", m),
        }
    }

    // Abrir archivo de salida para escritura asíncrona pieza a pieza
    let mut file = tokio::fs::OpenOptions::new()
        .write(true).create(true).truncate(true)
        .open(output_path).await?;

    // Solo acumulamos los hashes (32 bytes × num_pieces, nunca los datos completos)
    let mut piece_hashes: Vec<u8> = Vec::with_capacity(num_pieces * 32);
    let mut total_bytes: u64 = 0;

    for pi in 0..num_pieces {
        writer.send(Message::Request {
            file_index:  0,
            piece_index: pi as u32,
            begin:       0,
            length:      piece_length as u32,
        }).await?;

        let data = loop {
            match reader.next().await.unwrap()? {
                Message::KeepAlive => continue,
                Message::Piece { piece_index, begin: 0, data, .. } if piece_index == pi as u32 => {
                    break data;
                }
                Message::Reject { piece_index, .. } => {
                    anyhow::bail!("pieza {} rechazada", piece_index);
                }
                _ => continue,
            }
        };

        // Escribir al offset correcto y liberar los datos de inmediato
        file.seek(SeekFrom::Start(pi as u64 * piece_length as u64)).await?;
        file.write_all(&data).await?;
        total_bytes = pi as u64 * piece_length as u64 + data.len() as u64;

        piece_hashes.extend_from_slice(&hash_piece(&data));

        print!("\rpieza {}/{} descargada", pi + 1, num_pieces);
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }

    file.flush().await?;
    drop(file);
    println!("\nguardado en {}", output_path);
    println!("tamaño: {} bytes", total_bytes);

    // Verificar integridad: hash(concat(hashes_piezas)) == info_hash
    let computed_hash = hash_piece(&piece_hashes);
    if computed_hash == info_hash {
        println!("✅ verificación SHA-256 correcta");
    } else {
        println!("❌ verificación SHA-256 FALLÓ");
        println!("   esperado:  {}", info_hash_hex);
        println!("   calculado: {}", hex::encode(computed_hash));
    }

    writer.send(Message::Bye { reason: "done".into() }).await?;
    Ok(())
}
