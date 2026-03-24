# Arquitectura de quictorrent

## Grafo de dependencias entre crates

```
qt-protocol <- qt-transport <- qt-peer <- qt-client
                                  ^             ^
qt-pieces ────────────────────────┘             |
qt-tracker ─────────────────────────────────────┤
qt-dht ─────────────────────────────────────────┘
```

## Crates

### `qt-protocol`
Tipos y lógica del protocolo wire. No depende de red ni de disco.

- **Mensajes wire** — `Hello`, `HelloAck`, `HaveAll`, `HaveBitmap`, `Request`,
  `Piece`, `Reject`, `Cancel`, `PriorityHint`, `Bye`, `KeepAlive`
- **Framing** — length-prefix (4 bytes big-endian) con `MessageCodec` (tokio-util)
- **Metainfo** — `FileEntry`, `Metainfo`, cálculo de `info_hash` (SHA-256 de hashes de piezas)
- **Tamaño de pieza** — `piece_length_for_size(file_size)`:
  `next_pow2(file_size / 2048)`, acotado a [64 KiB, 512 MiB]; ≥ 1 TiB → 1 GiB.
  Produce ~2048 piezas en cada límite de tramo.

  | Archivo      | Pieza   | Bloques |
  |-------------|---------|---------|
  | ≤  128 MB   |  64 KB  |       4 |
  | ≤  256 MB   | 128 KB  |       8 |
  | ≤  512 MB   | 256 KB  |      16 |
  | ≤    1 GB   | 512 KB  |      32 |
  | ≤    2 GB   |   1 MB  |      64 |
  | ≤    4 GB   |   2 MB  |     128 |
  | ≤    8 GB   |   4 MB  |     256 |
  | ≤   16 GB   |   8 MB  |     512 |
  | ≤   32 GB   |  16 MB  |    1024 |
  | ≤   64 GB   |  32 MB  |    2048 |
  | ≤  128 GB   |  64 MB  |    4096 |
  | ≤  256 GB   | 128 MB  |    8192 |
  | ≤  512 GB   | 256 MB  |   16384 |
  |  < 1 TB     | 512 MB  |   32768 |
  |  ≥ 1 TB     |   1 GB  |   65536 |

- **Prioridades** — 9 niveles (`Minimum` → `Maximum`), codificadas en 1 byte
- **Autenticación** — `None`, `Credentials` (usuario/contraseña), `Token`, `Certificate` (mTLS X.509)

### `qt-transport`
Capa QUIC sobre [quinn](https://github.com/quinn-rs/quinn). TLS 1.3 obligatorio con certificado self-signed efímero.

- `QuicEndpoint` — abstracción de cliente y servidor QUIC
- Expone streams bidireccionales tipados que `qt-peer` consume con `MessageCodec`

### `qt-peer`
Sesión P2P para un único par. Gestiona el handshake y el estado de disponibilidad.

- Handshake: `Hello` → `HelloAck`
- Disponibilidad: `HaveAll` / `HaveBitmap` / `HaveNone`
- Descarga: `Request` / `Piece` / `Reject` / `Cancel`

### `qt-pieces`
Almacenamiento y verificación de piezas en disco.

- Escritura directa al offset correcto (no acumula en RAM)
- Verificación SHA-256 por pieza al recibirla
- Picker rarest-first para orden de solicitud

### `qt-tracker`
Tracker HTTP: announce/scrape con persistencia SQLite y TTL configurable.

### `qt-dht`
DHT Kademlia 256-bit para descubrimiento de peers sin tracker central.

- Routing table con buckets de 8 nodos
- Store con TTL y persistencia JSON entre sesiones

### `qt-client`
Binario CLI `qt` y herramientas de prueba `seed`/`download`.

- `seed` — semilla un archivo: calcula metainfo y sirve piezas vía QUIC
- `download` — descarga desde un seeder: escribe cada pieza a disco
  en cuanto llega (E/S asíncrona con `tokio::fs`), sin buffering en RAM

## Formato de transferencia

```
[4 bytes longitud] [N bytes payload MessagePack]
```

Cada pieza se identifica por `(file_index, piece_index, begin, length)`.
El `info_hash` es SHA-256 de la concatenación de los hashes SHA-256 de cada pieza.

## Flujo de descarga

```
Downloader                          Seeder
    |--- KeepAlive ------------------>|   (materializa stream QUIC)
    |--- Hello(info_hash, peer_id) -->|
    |<-- HelloAck(accepted=true) -----|
    |<-- HaveAll ----------------------|
    |--- Request(piece=0) ----------->|
    |<-- Piece(piece=0, data) ---------|
    |--- Request(piece=1) ----------->|
    |<-- Piece(piece=1, data) ---------|
    |         ...                     |
    |--- Bye ------------------------->|
```
