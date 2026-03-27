# bitturbulence

Implementación experimental del protocolo BitTurbulence: transferencia de archivos P2P sobre QUIC, escrita en Rust. Inspirada en BitTorrent. Básicamente es una reescritura de BitTorrent v2 en Rust usando Claude sobre el que quiero aplicar algunos cambios.
El objetivo es que funcione sobre QUIC, que la asignación del tamaño de pieza sea adaptativo según el tamaño de archivo (y que los archivos no compartan piezas) dentro de cada torrent —que aquí, de ahora en adelante, se llamarán flows (bitflows) —, que se pueda predefinir la prioridad de cada archivo dentro de un mismo flow al momento de crearlo y a la vez se pueda precisar mejor esa prioridad, que haya deduplicación entre flows que comparten piezas, que pueda ajustarse un ancho de banda por archivo (solo a nivel de cliente, no predefinido), que se puedan desarrollar plugins que cambien algunos comportamientos o añadan funciones e implementar mejoras para peers móviles o redes con restricciones (CG-NAT)

## Características

- **Transporte QUIC** — TLS 1.3 obligatorio, multiplexación de streams, sin head-of-line blocking
- **Protocolo wire propio** — mensajes binarios con framing length-prefixed
- **Piezas adaptativas** — tamaño de pieza calculado automáticamente según el tamaño del archivo (64 KB → 1 GB, siempre potencia de 2, ~2048 piezas por archivo)
- **Sin compartición de piezas entre archivos** — cada archivo tiene su propio espacio de piezas independiente
- **Descarga paralela** — múltiples streams QUIC por conexión, un bloque (16 KB) por stream; `BlockScheduler` rarest-first coordina hasta 4 streams simultáneos por peer
- **9 niveles de prioridad** — por archivo (Minimum → Maximum)
- **Verificación Merkle** — árbol de tres niveles (bloque → pieza → archivo), igual que BitTorrent v2
- **DHT Kademlia 256-bit** — descubrimiento de peers sin tracker central
- **Tracker QUIC propio** — announce/scrape sobre QUIC, con persistencia SQLite y TTL
- **Autenticación configurable** — None, Credentials, Token, mTLS (certificados X.509)

## Arquitectura
```
┌──────────────────────────────────────────────────────┐
│              bitturbulence-client (submarine)         │
│      CLI: submarine flow add/start/...               │
└──────┬──────────┬──────────┬───────────────────────┘
       │          │          │
  bitturbulence-peer  bitturbulence-tracker  bitturbulence-dht
       │          │          │
  bitturbulence-pieces  bitturbulence-transport (QUIC/quinn)
       │          │
  bitturbulence-protocol (mensajes wire, metainfo, auth)
```

### Crates

| Crate | Descripción |
|---|---|
| `bitturbulence-protocol` | Mensajes wire, metainfo BitFlow, info hash, prioridades, autenticación |
| `bitturbulence-transport` | Endpoint QUIC con quinn, TLS self-signed, streams tipados |
| `bitturbulence-peer` | Sesión P2P: Hello/HelloAck, estado de disponibilidad, requests |
| `bitturbulence-pieces` | Almacenamiento en disco, `BlockScheduler` rarest-first, verificación Merkle |
| `bitturbulence-tracker` | Tracker QUIC: announce/scrape, persistencia SQLite, TTL |
| `bitturbulence-dht` | DHT Kademlia 256-bit: routing table, store con TTL, persistencia JSON |
| `bitturbulence-client` | Binario CLI `submarine` y herramientas `seed`/`download` |

## Requisitos

- Rust 1.70+
- Linux/macOS (Windows no probado)

## Compilar
```bash
git clone https://github.com/jsunyermias/bitturbulence.git
cd bitturbulence
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
submarine flow add file.bitflow
submarine flow start <id>
submarine status
submarine serve
```

## Tests
```bash
cargo test --workspace
```

181 tests, 0 fallos.

## Estado del proyecto

Experimental. Transferencia P2P real verificada en red local (hasta 3 GB, SHA-256 correcto). Descarga paralela implementada y verificada con current de 8 peers.

**No listo para producción.** Faltan: NAT traversal, limitación de velocidad.

## Licencia

GPL-3.0
