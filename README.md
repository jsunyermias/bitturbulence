# quictorrent

Implementación experimental del protocolo BitTurbulence: transferencia de archivos P2P sobre QUIC, escrita en Rust. Inspirada en BitTorrent.

## Características

- **Transporte QUIC** — TLS 1.3 obligatorio, multiplexación de streams, sin head-of-line blocking
- **Protocolo wire propio** — mensajes binarios con framing length-prefixed
- **Piezas adaptativas** — tamaño de pieza calculado automáticamente según el tamaño del archivo (64 KB → 1 GB, siempre potencia de 2, ~2048 piezas por archivo)
- **Sin compartición de piezas entre archivos** — cada archivo tiene su propio espacio de piezas independiente
- **9 niveles de prioridad** — por archivo (Minimum → Maximum)
- **Verificación SHA-256** — integridad garantizada por pieza
- **DHT Kademlia 256-bit** — descubrimiento de peers sin tracker central
- **Tracker QUIC propio** — announce/scrape sobre QUIC, con persistencia SQLite y TTL
- **Autenticación configurable** — None, Credentials, Token, mTLS (certificados X.509)

## Arquitectura
```
┌─────────────────────────────────────────────┐
│                  qt-client                   │
│         CLI: qt torrent add/start/...        │
└──────┬──────────┬──────────┬────────────────┘
       │          │          │
  qt-peer    qt-tracker   qt-dht
       │          │          │
  qt-pieces  qt-transport (QUIC/quinn)
       │          │
  qt-protocol (mensajes wire, metainfo, auth)
```

### Crates

| Crate | Descripción |
|---|---|
| `qt-protocol` | Mensajes wire, metainfo, info hash SHA-256, prioridades, autenticación |
| `qt-transport` | Endpoint QUIC con quinn, TLS self-signed, streams tipados |
| `qt-peer` | Sesión P2P: Hello/HelloAck, estado de disponibilidad, requests |
| `qt-pieces` | Almacenamiento en disco, picker rarest-first, verificación SHA-256 |
| `qt-tracker` | Tracker HTTP: announce/scrape, persistencia SQLite, TTL |
| `qt-dht` | DHT Kademlia 256-bit: routing table, store con TTL, persistencia JSON |
| `qt-client` | Binario CLI `qt` y herramientas de prueba `seed`/`download` |

## Requisitos

- Rust 1.70+
- Linux/macOS (Windows no probado)

## Compilar
```bash
git clone https://github.com/jsunyermias/quictorrent.git
cd quictorrent
cargo build --release
```

## Uso rápido
```bash
# Compartir un archivo
./target/release/seed /path/to/file.mkv 7777

# Descargar un archivo
./target/release/download \
  192.168.1.100:7777 \
  <info_hash_hex> \
  <num_pieces> \
  <piece_length> \
  /path/to/output.mkv

# CLI completo
quictorrent torrent add file.turbulence
quictorrent torrent start <id>
quictorrent status
quictorrent serve
```

## Tests
```bash
cargo test --workspace
```

103 tests, 0 fallos.

## Estado del proyecto

Experimental. Transferencia P2P real verificada en red local (hasta 3 GB, SHA-256 correcto).

**No listo para producción.** Faltan: descarga paralela, NAT traversal, pipeline de requests, limitación de velocidad.

## Licencia

GPL-3.0
