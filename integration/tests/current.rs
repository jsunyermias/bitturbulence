//! Test de current P2P con 8 peers y distribución de piezas heterogénea.
//!
//! Simula las proporciones de un archivo de 10 GB con 8 peers:
//!
//! | Peer | Piezas iniciales | Porcentaje |
//! |------|-----------------|------------|
//! | 0    | 256 / 256        | 100%       |
//! | 1    | 128 / 256        | 50%        |
//! | 2    |  77 / 256        | 30%        |
//! | 3–7  |   0 / 256        |  0%        |
//!
//! Al finalizar, todos deben tener 100% de las piezas con SHA-256 correcto.
//!
//! # Arquitectura del test
//!
//! Para cada par (downloader → filler):
//! - Se abre una conexión QUIC
//! - El downloader abre streams bidi, uno por bloque pendiente
//! - El filler acepta streams y sirve bloques desde su store
//! - Un `BlockScheduler` compartido coordina qué bloques pide cada stream
//!
//! La topología de conexiones es:
//! - Todos los peers con datos incompletos (1–7) conectan a todos los que tienen piezas (0, 1, 2)
//! - Peer 1 y 2 también conectan a Peer 0 para completar sus propias piezas

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, atomic::{AtomicUsize, Ordering}},
    time::Duration,
};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tempfile::TempDir;
use tokio::sync::Mutex;
use tokio_util::codec::{FramedRead, FramedWrite};

use bitturbulence_pieces::{BlockScheduler, BlockTask, TorrentStore, hash_block, piece_root, file_root, verify_piece};
use bitturbulence_protocol::{
    AuthPayload, FileEntry, Message, MessageCodec, Metainfo, Priority,
    BLOCK_SIZE, PROTOCOL_VERSION,
};
use bitturbulence_transport::{PeerConnection, QuicEndpoint};

// ── Parámetros del test ───────────────────────────────────────────────────────

/// Número total de piezas (simula proporciones de 10 GB con ~2048 piezas).
const NUM_PIECES: usize = 256;

/// Tamaño de pieza en bytes (64 KB = 4 bloques de 16 KB).
const PIECE_LEN: u32 = 64 * 1024;

/// Tamaño total del archivo simulado (16 MB).
const FILE_SIZE: u64 = NUM_PIECES as u64 * PIECE_LEN as u64;

/// Número de peers en el current.
const NUM_PEERS: usize = 8;

/// Streams de datos simultáneos por conexión (como en daemon.rs).
const STREAMS_PER_CONN: usize = 4;

/// Timeout del test completo.
const TEST_TIMEOUT: Duration = Duration::from_secs(120);

// ── Distribución inicial de piezas ────────────────────────────────────────────

/// Cuántas piezas tiene cada peer al inicio.
fn initial_pieces(peer: usize) -> usize {
    match peer {
        0 => NUM_PIECES,        // 100%
        1 => NUM_PIECES / 2,    // 50%
        2 => NUM_PIECES * 3 / 10, // 30%
        _ => 0,                 // 0%
    }
}

// ── Estado compartido de un peer ──────────────────────────────────────────────

struct PeerState {
    store:      TorrentStore,
    scheduler:  Mutex<BlockScheduler>,
    have:       Mutex<Vec<bool>>,
    /// Piezas verificadas correctamente.
    verified:   AtomicUsize,
    piece_hashes: Vec<[u8; 32]>,
    info_hash:  [u8; 32],
    /// Directorio temporal (necesitamos mantenerlo vivo).
    _dir: TempDir,
}

impl PeerState {
    #[allow(dead_code)]
    fn num_pieces(&self) -> usize {
        NUM_PIECES
    }

    async fn our_bitfield(&self) -> Vec<bool> {
        self.have.lock().await.clone()
    }

    async fn is_complete(&self) -> bool {
        self.scheduler.lock().await.is_complete()
    }

    async fn verify_piece_and_mark(&self, pi: u32) -> bool {
        let data = match self.store.read_piece(0, pi).await {
            Ok(d) => d,
            Err(_) => {
                self.scheduler.lock().await.mark_piece_hash_failed(pi);
                return false;
            }
        };
        let expected = &self.piece_hashes[pi as usize];
        if verify_piece(&data, expected) {
            self.have.lock().await[pi as usize] = true;
            self.scheduler.lock().await.mark_piece_verified(pi);
            self.verified.fetch_add(1, Ordering::Relaxed);
            true
        } else {
            self.scheduler.lock().await.mark_piece_hash_failed(pi);
            false
        }
    }
}

// ── Generación de datos de prueba ─────────────────────────────────────────────

/// Genera los datos y hashes de todas las piezas.
/// Los datos son deterministas (basados en el índice de pieza).
fn generate_pieces() -> (Vec<Vec<u8>>, Vec<[u8; 32]>, [u8; 32]) {
    let mut pieces   = Vec::with_capacity(NUM_PIECES);
    let mut hashes   = Vec::with_capacity(NUM_PIECES);

    for pi in 0..NUM_PIECES {
        // Datos deterministas: byte = (pi * 7 + i) % 251
        let data: Vec<u8> = (0..PIECE_LEN as usize)
            .map(|i| ((pi * 7 + i) % 251) as u8)
            .collect();
        let h = piece_root(&data);
        pieces.push(data);
        hashes.push(h);
    }

    let info_hash = hash_block(&file_root(&hashes));
    (pieces, hashes, info_hash)
}

// ── Inicialización de un peer ──────────────────────────────────────────────────

async fn init_peer(
    peer_idx:     usize,
    pieces:       &[Vec<u8>],
    hashes:       &[[u8; 32]],
    info_hash:    [u8; 32],
    meta:         &Metainfo,
) -> Arc<PeerState> {
    let dir = TempDir::new().unwrap();
    let store = TorrentStore::open(dir.path(), meta).await.unwrap();

    let initial = initial_pieces(peer_idx);
    let is_filler = initial == NUM_PIECES;

    let piece_len  = meta.files[0].piece_length();
    let last_piece = meta.files[0].last_piece_length();
    let mut sched  = BlockScheduler::new(0, NUM_PIECES, piece_len, last_piece);

    let mut have = vec![false; NUM_PIECES];

    // Pre-escribir piezas iniciales en disco
    for pi in 0..initial {
        let data = &pieces[pi];
        // Escribir bloque a bloque
        let mut offset = 0u32;
        while offset < data.len() as u32 {
            let chunk_len = BLOCK_SIZE.min(data.len() as u32 - offset);
            store.write_block(0, pi as u32, offset, &data[offset as usize..(offset + chunk_len) as usize]).await.unwrap();
            offset += chunk_len;
        }
        sched.mark_piece_verified(pi as u32);
        have[pi] = true;
    }

    if is_filler {
        // Filler puro: el scheduler está completo desde el inicio
        assert!(sched.is_complete());
    }

    Arc::new(PeerState {
        store,
        scheduler: Mutex::new(sched),
        have: Mutex::new(have),
        verified: AtomicUsize::new(initial),
        piece_hashes: hashes.to_vec(),
        info_hash,
        _dir: dir,
    })
}

// ── Tarea filler (acepta un stream y sirve bloques) ───────────────────────────

async fn handle_seeder_stream(
    conn_send: quinn::SendStream,
    conn_recv: quinn::RecvStream,
    state:     Arc<PeerState>,
) {
    let mut writer = FramedWrite::new(conn_send, MessageCodec);
    let mut reader = FramedRead::new(conn_recv, MessageCodec);

    loop {
        match reader.next().await {
            None | Some(Err(_)) => break,
            Some(Ok(Message::Request { file_index, piece_index, begin, length })) => {
                let have = state.have.lock().await;
                let pi   = piece_index as usize;
                if have.get(pi).copied().unwrap_or(false) {
                    drop(have);
                    match state.store.read_block(file_index as usize, piece_index, begin, length).await {
                        Ok(data) => {
                            let _ = writer.send(Message::Piece {
                                file_index, piece_index, begin,
                                data: Bytes::from(data),
                            }).await;
                        }
                        Err(_) => {
                            let _ = writer.send(Message::Reject {
                                file_index, piece_index, begin, length,
                            }).await;
                        }
                    }
                } else {
                    drop(have);
                    let _ = writer.send(Message::Reject {
                        file_index, piece_index, begin, length,
                    }).await;
                }
            }
            Some(Ok(_)) => {}
        }
    }
}

// ── Aceptar loop de un peer ────────────────────────────────────────────────────

async fn accept_loop(endpoint: Arc<QuicEndpoint>, state: Arc<PeerState>) {
    loop {
        match endpoint.accept().await {
            None => break,
            Some(Err(_)) => continue,
            Some(Ok(conn)) => {
                let state2 = state.clone();
                tokio::spawn(async move {
                    // Stream de handshake
                    let Ok((mut hello_w, mut hello_r)) = conn.accept_bidi_stream().await else { return };
                    let hello_w2 = &mut hello_w;
                    let hello_r2 = &mut hello_r;

                    // Leer Hello
                    let msg = hello_r2.next().await;
                    let Some(Ok(Message::Hello { info_hash, .. })) = msg else { return };
                    if info_hash != state2.info_hash { return; }

                    // Enviar HelloAck
                    let peer_id = [0x53u8; 32];
                    let _ = hello_w2.send(Message::HelloAck {
                        peer_id, accepted: true, reason: None,
                    }).await;

                    // Stream de control: enviar disponibilidad y aceptar data streams
                    let Ok((mut ctrl_w, _ctrl_r)) = conn.accept_bidi_stream().await else { return };

                    let bits = state2.our_bitfield().await;
                    let msg = if bits.iter().all(|&h| h) {
                        Message::HaveAll { file_index: 0 }
                    } else if bits.iter().all(|&h| !h) {
                        Message::HaveNone { file_index: 0 }
                    } else {
                        let mut bytes = vec![0u8; bits.len().div_ceil(8)];
                        for (i, &has) in bits.iter().enumerate() {
                            if has { bytes[i / 8] |= 0x80 >> (i % 8); }
                        }
                        Message::HaveBitmap {
                            file_index: 0,
                            bitmap: Bytes::from(bytes),
                        }
                    };
                    let _ = ctrl_w.send(msg).await;

                    // Aceptar data streams
                    loop {
                        match conn.accept_bidi_stream().await {
                            Ok((w, r)) => {
                                tokio::spawn(handle_seeder_stream(w.into_inner(), r.into_inner(), state2.clone()));
                            }
                            Err(_) => break,
                        }
                    }
                });
            }
        }
    }
}

// ── Descarga desde un filler ───────────────────────────────────────────────────

async fn download_from_peer(
    conn:     PeerConnection,
    state:    Arc<PeerState>,
    peer_id:  [u8; 32],
) {
    // Handshake
    let Ok((mut hello_w, mut hello_r)) = conn.open_bidi_stream().await else { return };

    let _ = hello_w.send(Message::Hello {
        version:   PROTOCOL_VERSION,
        peer_id,
        info_hash: state.info_hash,
        auth:      AuthPayload::None,
    }).await;

    // Esperar HelloAck
    loop {
        match hello_r.next().await {
            Some(Ok(Message::HelloAck { accepted: true, .. })) => break,
            Some(Ok(Message::HelloAck { accepted: false, .. })) => return,
            Some(Ok(_)) => continue,
            _ => return,
        }
    }
    drop(hello_w);
    drop(hello_r);

    // Stream de control: recibir disponibilidad del filler
    let Ok((mut ctrl_w, mut ctrl_r)) = conn.open_bidi_stream().await else { return };

    // Anunciar nuestra disponibilidad al filler
    let our_bits = state.our_bitfield().await;
    let msg = if our_bits.iter().all(|&h| !h) {
        Message::HaveNone { file_index: 0 }
    } else {
        let mut bytes = vec![0u8; our_bits.len().div_ceil(8)];
        for (i, &has) in our_bits.iter().enumerate() {
            if has { bytes[i / 8] |= 0x80 >> (i % 8); }
        }
        Message::HaveBitmap {
            file_index: 0,
            bitmap: Bytes::from(bytes),
        }
    };
    let _ = ctrl_w.send(msg).await;

    // Leer disponibilidad del filler
    let seeder_bits: Vec<bool> = loop {
        match ctrl_r.next().await {
            Some(Ok(Message::HaveAll { .. })) => break vec![true; NUM_PIECES],
            Some(Ok(Message::HaveNone { .. })) => return, // filler no tiene nada
            Some(Ok(Message::HaveBitmap { bitmap, .. })) => {
                let bits: Vec<bool> = (0..NUM_PIECES)
                    .map(|i| bitmap.get(i / 8).is_some_and(|b| (b >> (7 - (i % 8))) & 1 == 1))
                    .collect();
                break bits;
            }
            Some(Ok(_)) => continue,
            _ => return,
        }
    };

    // Registrar disponibilidad del filler en el scheduler
    state.scheduler.lock().await.add_peer_bitfield(&seeder_bits);

    // Abrir streams de datos en paralelo
    let (result_tx, mut result_rx) = tokio::sync::mpsc::channel::<(u32, u32, bool)>(64);

    let mut active_streams: usize = 0;
    let mut _total_requested: usize = 0;

    'outer: loop {
        // Abrir nuevos streams hasta el límite mientras haya bloques pendientes
        while active_streams < STREAMS_PER_CONN {
            let task_opt = state.scheduler.lock().await.schedule_pending(&seeder_bits);
            let Some(BlockTask { fi, pi, bi, begin, length }) = task_opt else { break };

            let Ok((w, r)) = conn.open_bidi_stream().await else {
                state.scheduler.lock().await.mark_block_failed(pi, bi);
                break;
            };

            let result_tx2 = result_tx.clone();
            let state2     = state.clone();
            active_streams += 1;
            _total_requested += 1;

            tokio::spawn(async move {
                let mut writer = FramedWrite::new(w.into_inner(), MessageCodec);
                let mut reader = FramedRead::new(r.into_inner(), MessageCodec);

                let ok = if writer.send(Message::Request {
                    file_index: fi as u16,
                    piece_index: pi,
                    begin,
                    length,
                }).await.is_ok() {
                    loop {
                        match reader.next().await {
                            Some(Ok(Message::Piece { piece_index, begin: b, data, .. }))
                                if piece_index == pi && b == begin =>
                            {
                                let wrote = state2.store
                                    .write_block(fi, pi, begin, &data)
                                    .await
                                    .is_ok();
                                break wrote;
                            }
                            Some(Ok(Message::Reject { piece_index, begin: b, .. }))
                                if piece_index == pi && b == begin => break false,
                            None | Some(Err(_)) => break false,
                            _ => continue,
                        }
                    }
                } else {
                    false
                };

                let _ = result_tx2.send((pi, bi, ok)).await;
            });
        }

        if active_streams == 0 {
            // No hay streams activos ni pendientes para este filler
            break 'outer;
        }

        // Esperar resultado de un stream
        let Some((pi, bi, ok)) = result_rx.recv().await else { break };
        active_streams -= 1;

        if ok {
            let piece_done = state.scheduler.lock().await.mark_block_done(pi, bi, [0u8; 32]);
            if piece_done {
                state.verify_piece_and_mark(pi).await;
            }
        } else {
            state.scheduler.lock().await.mark_block_failed(pi, bi);
        }
    }

    // Quitar aportación del filler al scheduler
    state.scheduler.lock().await.remove_peer_bitfield(&seeder_bits);
}

// ── Test principal ────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn current_8_peers_mixed_availability() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    // ── Generar datos ──────────────────────────────────────────────────────
    let (pieces, hashes, info_hash) = generate_pieces();

    let meta = Metainfo {
        name:      "current-test".into(),
        info_hash,
        files: vec![
            FileEntry {
                path:         vec!["data.bin".into()],
                size:         FILE_SIZE,
                piece_hashes: hashes.clone(),
                priority:     Priority::Normal,
            }
        ],
        trackers: vec![],
        comment:  None,
    };

    // ── Inicializar peers ──────────────────────────────────────────────────
    let mut states: Vec<Arc<PeerState>> = Vec::with_capacity(NUM_PEERS);
    for i in 0..NUM_PEERS {
        let s = init_peer(i, &pieces, &hashes, info_hash, &meta).await;
        states.push(s);
    }

    // ── Vincular endpoints QUIC ────────────────────────────────────────────
    let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let mut endpoints: Vec<Arc<QuicEndpoint>> = Vec::with_capacity(NUM_PEERS);
    let mut addrs:     Vec<SocketAddr>         = Vec::with_capacity(NUM_PEERS);

    for _ in 0..NUM_PEERS {
        let ep   = Arc::new(QuicEndpoint::bind(bind).unwrap());
        let addr = ep.local_addr().unwrap();
        endpoints.push(ep);
        addrs.push(addr);
    }

    // ── Arrancar accept loops ──────────────────────────────────────────────
    for (ep, state) in endpoints.iter().zip(states.iter()) {
        tokio::spawn(accept_loop(ep.clone(), state.clone()));
    }

    // Pequeña pausa para que los accept loops estén listos
    tokio::time::sleep(Duration::from_millis(50)).await;

    // ── Conectar drainers a fillers ─────────────────────────────────────
    //
    // Topología:
    // - Peer 1 (50%) → Peer 0 (100%)
    // - Peer 2 (30%) → Peer 0 (100%)
    // - Peers 3–7 (0%) → Peers 0, 1, 2 (en paralelo)
    //
    // Esto ejercita rarest-first: los 5 drainers ven la misma disponibilidad
    // y priorizan piezas raras (las que solo Peer 2 tiene, etc.).

    let mut download_tasks = vec![];

    let sources: HashMap<usize, Vec<usize>> = {
        let mut m = HashMap::new();
        m.insert(1, vec![0]);       // Peer 1 descarga de Peer 0
        m.insert(2, vec![0]);       // Peer 2 descarga de Peer 0
        for p in 3..NUM_PEERS {
            m.insert(p, vec![0, 1, 2]); // Leechers descargan de 0, 1 y 2
        }
        m
    };

    for (&downloader, fillers) in &sources {
        for &filler in fillers {
            let state  = states[downloader].clone();
            let ep     = endpoints[downloader].clone();
            let addr   = addrs[filler];
            let mut peer_id = [0u8; 32];
            peer_id[0] = downloader as u8;

            let task = tokio::spawn(async move {
                match ep.connect(addr).await {
                    Ok(conn) => download_from_peer(conn, state, peer_id).await,
                    Err(e)   => eprintln!("connect {downloader}→{filler}: {e:?}"),
                }
            });
            download_tasks.push(task);
        }
    }

    // ── Esperar a que todos los peers completen ────────────────────────────
    let result = tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            let all_done = {
                let mut done = true;
                for (i, s) in states.iter().enumerate() {
                    if !s.is_complete().await {
                        done = false;
                        let pct = s.verified.load(Ordering::Relaxed) * 100 / NUM_PIECES;
                        eprintln!("peer {i}: {pct}%");
                    }
                }
                done
            };
            if all_done { break; }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }).await;

    // ── Resultado ──────────────────────────────────────────────────────────
    for task in download_tasks {
        task.await.ok();
    }

    assert!(result.is_ok(), "timeout: no todos los peers completaron en {}s", TEST_TIMEOUT.as_secs());

    // ── Verificar integridad final de todos los peers ──────────────────────
    for (i, state) in states.iter().enumerate() {
        assert!(
            state.is_complete().await,
            "peer {i} no está completo ({}/{} piezas)",
            state.verified.load(Ordering::Relaxed),
            NUM_PIECES,
        );

        // Verificar SHA-256 de cada pieza en disco
        for pi in 0..NUM_PIECES as u32 {
            let data = state.store.read_piece(0, pi).await
                .unwrap_or_else(|_| panic!("peer {i}: no se pudo leer pieza {pi}"));
            assert!(
                verify_piece(&data, &hashes[pi as usize]),
                "peer {i}: pieza {pi} SHA-256 incorrecto"
            );
        }
        eprintln!("peer {i}: OK ({NUM_PIECES} piezas, SHA-256 correcto)");
    }
}
