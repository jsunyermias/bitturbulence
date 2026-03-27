# Issues abiertos — bitturbulence

Ordenados por prioridad de implementación. Actualizado: 2026-03-27.

| # | Título | Prioridad |
|---|--------|-----------|
| [#44](https://github.com/jsunyermias/bitturbulence/issues/44) | Logging estructurado: tracing spans con peer_id/addr/info_hash por conexión | 🟠 Media-alta |
| [#38](https://github.com/jsunyermias/bitturbulence/issues/38) | Filler: procesar mensajes Cancel para evitar servir bloques ya cancelados | 🟠 Media-alta |
| [#41](https://github.com/jsunyermias/bitturbulence/issues/41) | Límite de conexiones simultáneas de peers + evicción LRU | 🟠 Media-alta |
| [#43](https://github.com/jsunyermias/bitturbulence/issues/43) | Validar longitud de bloque en mensajes Piece recibidos | 🟠 Media |
| [#42](https://github.com/jsunyermias/bitturbulence/issues/42) | Blacklisting de peers por mismatches de hash Merkle reiterados | 🟠 Media |
| [#40](https://github.com/jsunyermias/bitturbulence/issues/40) | Backoff exponencial en reconexión de peers (1s → 2s → 4s … 60s) | 🟠 Media |
| [#45](https://github.com/jsunyermias/bitturbulence/issues/45) | Métricas de throughput: velocidad (B/s) por peer y global | 🟠 Media |
| [#39](https://github.com/jsunyermias/bitturbulence/issues/39) | PriorityHint: reordenar descarga del scheduler al recibir cambio de prioridad | 🟠 Media |
| [#46](https://github.com/jsunyermias/bitturbulence/issues/46) | Progreso por archivo en respuesta IPC Status (% completado por file_index) | 🟡 Media-baja |
| [#23](https://github.com/jsunyermias/bitturbulence/issues/23) | `submarine status`: progreso en tiempo real (velocidad, ETA, peers activos) | 🟡 Media-baja |
| [#30](https://github.com/jsunyermias/bitturbulence/issues/30) | Velocímetro: estadísticas de velocidad en tiempo real | 🟡 Media-baja |
| [#47](https://github.com/jsunyermias/bitturbulence/issues/47) | Hot-reload de config del daemon sin reinicio (SIGHUP o IPC reload) | 🟡 Media-baja |
| [#18](https://github.com/jsunyermias/bitturbulence/issues/18) | NAT traversal: STUN + UDP hole punching + relay TURN | 🟡 Media-baja |
| [#13](https://github.com/jsunyermias/bitturbulence/issues/13) | Cliente tracker HTTP/UDP estándar (compat. BitTorrent) | 🟡 Baja |
| [#33](https://github.com/jsunyermias/bitturbulence/issues/33) | Streaming: prioridad automática de primeras piezas para reproducción progresiva | 🔵 Baja |
| [#8](https://github.com/jsunyermias/bitturbulence/issues/8) | Choke/unchoke: gestión de slots de subida | 🔵 Baja |
| [#20](https://github.com/jsunyermias/bitturbulence/issues/20) | Limitación de velocidad: throttling de subida/bajada | 🔵 Baja |
| [#7](https://github.com/jsunyermias/bitturbulence/issues/7) | Pool de conexiones de peers con límite global | 🔵 Baja |
| [#22](https://github.com/jsunyermias/bitturbulence/issues/22) | Vortex links: compartir flows sin `.bitflow` (`vortex://`) | 🔵 Baja |
| [#37](https://github.com/jsunyermias/bitturbulence/issues/37) | Web UI: panel de control con progreso en tiempo real via WebSocket | 🔵 Baja |
| [#25](https://github.com/jsunyermias/bitturbulence/issues/25) | Renombrar repositorio de quictorrent a bitturbulence | 🔵 Admin |
| [#34](https://github.com/jsunyermias/bitturbulence/issues/34) | Soporte Windows: compilación y CI en Windows | 🔵 Baja |
| [#35](https://github.com/jsunyermias/bitturbulence/issues/35) | Packaging: .deb, .rpm, Flatpak y Homebrew | 🔵 Baja |

## Cerrados recientemente

| # | Título | Commit |
|---|--------|--------|
| [#27](https://github.com/jsunyermias/bitturbulence/issues/27) | CI pipeline: clippy --deny warnings + cargo test + rustfmt | f4f3458 |
| [#32](https://github.com/jsunyermias/bitturbulence/issues/32) | Verificación Merkle incremental via HashRequest/HashResponse | 1b5d3ee |
| [#26](https://github.com/jsunyermias/bitturbulence/issues/26) | Multiplexación adaptativa: scale-down de data streams | 72f5a65 |
| [#28](https://github.com/jsunyermias/bitturbulence/issues/28) | Daemon en background + IPC Unix socket | 4f3f8c8 |
| [#21](https://github.com/jsunyermias/bitturbulence/issues/21) | `submarine flow create`: herramienta nativa Rust para generar `.bitflow` | 9fd9553 |
| — | Batería de tests: scheduler_actor, context, have_persist (154 tests total) | 779619d |
| [#36](https://github.com/jsunyermias/bitturbulence/issues/36) | Seeder automático: pasar a modo Seeding al completar la descarga | f4c90ee |
| [#29](https://github.com/jsunyermias/bitturbulence/issues/29) | Integrar DHT con descubrimiento de peers en el daemon | c9a8746 |
| [#31](https://github.com/jsunyermias/bitturbulence/issues/31) | Propagación de HavePiece a todos los peers conectados | e3de67d |
| [#19](https://github.com/jsunyermias/bitturbulence/issues/19) | Reanudación de descargas: persistencia del have bitfield | 6723f3b |
