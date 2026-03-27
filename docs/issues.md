# Issues abiertos — bitturbulence

Ordenados por prioridad de implementación. Actualizado: 2026-03-27 (issues #88–#137 añadidos, gap analysis vs libtorrent). Última revisión completa: 2026-03-27 (commit d6c705f).

| # | Título | Prioridad |
|---|--------|-----------|
| [#48](https://github.com/jsunyermias/bitturbulence/issues/48) | `submarine flow set-priority`: IPC + CLI para cambiar prioridad de archivo en caliente | 🟠 Media-alta |
| [#49](https://github.com/jsunyermias/bitturbulence/issues/49) | `flow create --priority <fi>:<level>`: predefinir prioridades al generar el .bitflow | 🟠 Media-alta |
| [#50](https://github.com/jsunyermias/bitturbulence/issues/50) | Coordinador multi-archivo por prioridad: drainer elige archivos según Priority antes de piezas | 🟠 Media-alta |
| [#54](https://github.com/jsunyermias/bitturbulence/issues/54) | Token bucket por archivo en FlowCtx: tasa de descarga configurable (bytes/s) | 🟠 Media |
| [#55](https://github.com/jsunyermias/bitturbulence/issues/55) | IPC/CLI `submarine flow throttle <flow-id> <fi> <bytes/s>`: ajustar throttle en caliente | 🟠 Media |
| [#56](https://github.com/jsunyermias/bitturbulence/issues/56) | Aplicar throttle en el drainer: respetar token bucket entre Requests por archivo | 🟠 Media |
| [#65](https://github.com/jsunyermias/bitturbulence/issues/65) | PEX (Peer Exchange): mensaje Wire para intercambiar listas de peers entre conexiones activas | 🟠 Media-alta |
| [#69](https://github.com/jsunyermias/bitturbulence/issues/69) | Selección de archivos: marcar archivos de un flow como "skip" — no descargar ni seedear | 🟠 Media-alta |
| [#71](https://github.com/jsunyermias/bitturbulence/issues/71) | Ratio tracking: acumular bytes subidos/bajados por flow y globalmente en estado persistente | 🟠 Media-alta |
| [#78](https://github.com/jsunyermias/bitturbulence/issues/78) | Magnet links: URI `magnet:?xt=urn:bttb:<info_hash>` para compartir flows sin archivo | 🟠 Media-alta |
| [#88](https://github.com/jsunyermias/bitturbulence/issues/88) | BEP 6 Fast Extension: mensajes AllowedFast + SuggestPiece para piezas servibles sin unchoke | 🟠 Media-alta |
| [#89](https://github.com/jsunyermias/bitturbulence/issues/89) | BEP 12 Multitracker: múltiples trackers por flow con tiers y fallback automático | 🟠 Media-alta |
| [#90](https://github.com/jsunyermias/bitturbulence/issues/90) | BEP 27 Torrents privados: flag `private` desactiva DHT y PEX para trackers privados | 🟠 Media-alta |
| [#91](https://github.com/jsunyermias/bitturbulence/issues/91) | Peer snubbing: marcar peer como snubbed si no envía bloque en 60 s y deprioritizarlo | 🟠 Media-alta |
| [#66](https://github.com/jsunyermias/bitturbulence/issues/66) | LPD (Local Peer Discovery): descubrir peers en la misma LAN via UDP multicast | 🟠 Media |
| [#67](https://github.com/jsunyermias/bitturbulence/issues/67) | UPnP / NAT-PMP: mapeo automático de puerto en el router para aceptar conexiones entrantes | 🟠 Media |
| [#68](https://github.com/jsunyermias/bitturbulence/issues/68) | Modo secuencial: descargar piezas en orden ascendente (previsualización mientras descarga) | 🟠 Media |
| [#74](https://github.com/jsunyermias/bitturbulence/issues/74) | Pre-allocación de disco: reservar el espacio total del flow antes de empezar la descarga | 🟠 Media |
| [#75](https://github.com/jsunyermias/bitturbulence/issues/75) | Nombre de archivo incompleto: guardar como `nombre.!bt` durante descarga, renombrar al completar | 🟠 Media |
| [#92](https://github.com/jsunyermias/bitturbulence/issues/92) | Parole mode: tras pieza corrupta, verificar peer con un bloque aislado antes de reincorporarlo | 🟠 Media |
| [#93](https://github.com/jsunyermias/bitturbulence/issues/93) | Seed mode rápido: arrancar en modo Seeding sin re-verificar disco (confiar en have.json) | 🟠 Media |
| [#94](https://github.com/jsunyermias/bitturbulence/issues/94) | Estado de sesión completo: serializar prioridades, tracker activo y stats (más allá del have bitfield) | 🟠 Media |
| [#95](https://github.com/jsunyermias/bitturbulence/issues/95) | Upload slot auto-optimization: calcular número óptimo de slots unchoked según capacidad de subida | 🟠 Media |
| [#118](https://github.com/jsunyermias/bitturbulence/issues/118) | Prioridad de archivo fine-grained: niveles 0–7 por archivo (no solo skip/normal/alta) | 🟠 Media |
| [#122](https://github.com/jsunyermias/bitturbulence/issues/122) | Límite de ancho de banda por flow: tasa upload/download configurable por flow independiente del límite global | 🟠 Media |
| [#123](https://github.com/jsunyermias/bitturbulence/issues/123) | Force recheck: re-verificar todos los archivos de un flow contra sus hashes desde CLI/IPC | 🟠 Media |
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
| [#57](https://github.com/jsunyermias/bitturbulence/issues/57) | STUN client: resolver IP pública y puerto externo (RFC 5389) | 🟡 Media-baja |
| [#58](https://github.com/jsunyermias/bitturbulence/issues/58) | Hole punching coordinado via tracker: intercambio de candidatos antes de conectar | 🟡 Media-baja |
| [#59](https://github.com/jsunyermias/bitturbulence/issues/59) | TURN relay fallback cuando hole punching falla (RFC 5766) | 🟡 Media-baja |
| [#61](https://github.com/jsunyermias/bitturbulence/issues/61) | QUIC connection migration: mantener sesión al cambiar de red (WiFi ↔ 4G) | 🟡 Media-baja |
| [#51](https://github.com/jsunyermias/bitturbulence/issues/51) | Índice global de bloques por hash: content-addressable store para deduplicación | 🟡 Media-baja |
| [#52](https://github.com/jsunyermias/bitturbulence/issues/52) | Deduplicación al añadir flow: detectar y marcar piezas ya presentes en disco | 🟡 Media-baja |
| [#53](https://github.com/jsunyermias/bitturbulence/issues/53) | Reference counting de bloques compartidos: no borrar datos al eliminar un flow | 🟡 Media-baja |
| [#70](https://github.com/jsunyermias/bitturbulence/issues/70) | Cola global de flows: estado Queued + lógica start/pause para respetar límite de activos | 🟡 Media-baja |
| [#72](https://github.com/jsunyermias/bitturbulence/issues/72) | Límite de ratio: pausar seeding automáticamente al alcanzar ratio objetivo | 🟡 Media-baja |
| [#73](https://github.com/jsunyermias/bitturbulence/issues/73) | Optimistic unchoke: desbloquear un peer aleatorio periódicamente para explorar capacidad | 🟡 Media-baja |
| [#76](https://github.com/jsunyermias/bitturbulence/issues/76) | Mover al completar: mover archivos a directorio destino configurable tras verificación | 🟡 Media-baja |
| [#77](https://github.com/jsunyermias/bitturbulence/issues/77) | Watch folder: monitorizar directorio y añadir automáticamente .bitflow nuevos | 🟡 Media-baja |
| [#81](https://github.com/jsunyermias/bitturbulence/issues/81) | IP filter / blocklist: rechazar conexiones de rangos de IP configurados (formato PeerGuardian) | 🟡 Media-baja |
| [#84](https://github.com/jsunyermias/bitturbulence/issues/84) | Webhook on complete: llamar URL configurable (POST JSON) al terminar un flow | 🟡 Media-baja |
| [#96](https://github.com/jsunyermias/bitturbulence/issues/96) | BEP 21 Partial seeds: anunciar al tracker qué archivos del flow están disponibles para subir | 🟡 Media-baja |
| [#97](https://github.com/jsunyermias/bitturbulence/issues/97) | BEP 42 DHT Security: derivar node ID desde IP del nodo para resistir ataques Sybil | 🟡 Media-baja |
| [#98](https://github.com/jsunyermias/bitturbulence/issues/98) | BEP 44 DHT Mutable Items: almacenar y recuperar datos mutables/inmutables firmados en el DHT | 🟡 Media-baja |
| [#99](https://github.com/jsunyermias/bitturbulence/issues/99) | BEP 43 DHT read-only: modo lectura para nodos móviles/embebidos (no anunciar en routing table) | 🟡 Media-baja |
| [#100](https://github.com/jsunyermias/bitturbulence/issues/100) | BEP 55 Holepunch extension: coordinar hole-punching entre dos peers via rendezvous peer | 🟡 Media-baja |
| [#101](https://github.com/jsunyermias/bitturbulence/issues/101) | BEP 47 Atributos de archivo: soporte de padding files, symlinks y bits de permisos en metainfo | 🟡 Media-baja |
| [#102](https://github.com/jsunyermias/bitturbulence/issues/102) | Peer turnover: desconectar proactivamente peers lentos/inactivos para probar nuevos candidatos | 🟡 Media-baja |
| [#103](https://github.com/jsunyermias/bitturbulence/issues/103) | Tracker announce strategy: opción de anunciar a todos los tiers en paralelo vs. parar en el primero | 🟡 Media-baja |
| [#104](https://github.com/jsunyermias/bitturbulence/issues/104) | Scrape en el cliente: consultar seeders/leechers de un swarm antes de conectar | 🟡 Media-baja |
| [#105](https://github.com/jsunyermias/bitturbulence/issues/105) | Peer classes: agrupar peers por rango de IP y aplicar rate limits o prioridad diferente | 🟡 Media-baja |
| [#116](https://github.com/jsunyermias/bitturbulence/issues/116) | Auto-gestión de flows: daemon inicia/detiene flows automáticamente según ratio y tiempo de seed | 🟡 Media-baja |
| [#124](https://github.com/jsunyermias/bitturbulence/issues/124) | Prioridad a nivel de pieza: asignar prioridad 0–7 a piezas individuales (más fino que por archivo) | 🟡 Media-baja |
| [#125](https://github.com/jsunyermias/bitturbulence/issues/125) | Renombrar archivo dentro de un flow en caliente sin detener ni mover el flow | 🟡 Media-baja |
| [#126](https://github.com/jsunyermias/bitturbulence/issues/126) | Mover storage en caliente: relocalizar archivos de un flow mientras está activo (descargando o seeding) | 🟡 Media-baja |
| [#127](https://github.com/jsunyermias/bitturbulence/issues/127) | Inyectar datos locales: añadir pieza desde disco como si se hubiera descargado (cross-seeding sin red) | 🟡 Media-baja |
| [#128](https://github.com/jsunyermias/bitturbulence/issues/128) | Manual peer connect: forzar conexión a IP:puerto específico desde CLI sin esperar al tracker/DHT | 🟡 Media-baja |
| [#129](https://github.com/jsunyermias/bitturbulence/issues/129) | Metadatos en .bitflow: campos comment, creator y creation_date en el metainfo | 🟡 Media-baja |
| [#130](https://github.com/jsunyermias/bitturbulence/issues/130) | Detección de flows duplicados: alertar y rechazar cuando se añade un flow con info_hash ya existente | 🟡 Media-baja |
| [#131](https://github.com/jsunyermias/bitturbulence/issues/131) | BEP 38 Collections: campo de colección en el metainfo para agrupar flows relacionados | 🟡 Media-baja |
| [#132](https://github.com/jsunyermias/bitturbulence/issues/132) | Re-announce manual: forzar anuncio inmediato a tracker/DHT/LSD desde CLI sin esperar el intervalo | 🟡 Media-baja |
| [#133](https://github.com/jsunyermias/bitturbulence/issues/133) | Upload mode automático: cuando el disco está lleno, pausar descarga y continuar subiendo | 🟡 Media-baja |
| [#134](https://github.com/jsunyermias/bitturbulence/issues/134) | Persistencia de lista de peers: guardar peers conocidos en el estado de sesión para reconectar más rápido | 🟡 Media-baja |
| [#135](https://github.com/jsunyermias/bitturbulence/issues/135) | BEP 9 ut_metadata: descargar metainfo .bitflow directamente desde peers del swarm (no solo tracker/DHT) | 🟡 Media-baja |
| [#18](https://github.com/jsunyermias/bitturbulence/issues/18) | NAT traversal (épico, ver #57–#59 para subissues) | 🟡 Media-baja |
| [#33](https://github.com/jsunyermias/bitturbulence/issues/33) | Streaming: prioridad automática de primeras piezas para reproducción progresiva | 🔵 Baja |
| [#8](https://github.com/jsunyermias/bitturbulence/issues/8) | Choke/unchoke: gestión de slots de subida | 🔵 Baja |
| [#62](https://github.com/jsunyermias/bitturbulence/issues/62) | Trait `BitTurbulencePlugin` con hooks: on_piece_verified, on_peer_connected, on_flow_complete | 🔵 Baja |
| [#63](https://github.com/jsunyermias/bitturbulence/issues/63) | Carga dinámica de plugins (.so / cdylib) vía libloading | 🔵 Baja |
| [#64](https://github.com/jsunyermias/bitturbulence/issues/64) | Plugin de ejemplo: logger de estadísticas (progreso, velocidad, peers) a CSV/JSON | 🔵 Baja |
| [#82](https://github.com/jsunyermias/bitturbulence/issues/82) | SOCKS5 / HTTP proxy: enrutar conexiones QUIC a través de proxy configurable | 🔵 Baja |
| [#83](https://github.com/jsunyermias/bitturbulence/issues/83) | Peer scoring: penalizar peers lentos/corruptos y priorizar los más rápidos | 🔵 Baja |
| [#85](https://github.com/jsunyermias/bitturbulence/issues/85) | RSS auto-download: suscripción a feeds RSS/Atom con filtro por título para añadir flows | 🔵 Baja |
| [#86](https://github.com/jsunyermias/bitturbulence/issues/86) | Prometheus metrics: endpoint `/metrics` con contadores de bytes, peers, flows activos | 🔵 Baja |
| [#87](https://github.com/jsunyermias/bitturbulence/issues/87) | Super-seeding (BEP 16): modo inicial que maximiza distribución sirviendo cada pieza una sola vez | 🔵 Baja |
| [#106](https://github.com/jsunyermias/bitturbulence/issues/106) | Modo anónimo: peer ID aleatorio por flow, no revelar fingerprint del cliente | 🔵 Baja |
| [#107](https://github.com/jsunyermias/bitturbulence/issues/107) | I2P transport: soporte de red I2P como capa de transporte para descargas anónimas | 🔵 Baja |
| [#108](https://github.com/jsunyermias/bitturbulence/issues/108) | BEP 54 lt_donthave: mensaje wire para notificar a un peer que descartaste una pieza | 🔵 Baja |
| [#109](https://github.com/jsunyermias/bitturbulence/issues/109) | BEP 33 DHT Scrape: obtener estadísticas de swarm (seeders/leechers) desde nodos DHT | 🔵 Baja |
| [#110](https://github.com/jsunyermias/bitturbulence/issues/110) | BEP 35 Torrent Signing: firmar metainfo del flow con keypair Ed25519 | 🔵 Baja |
| [#111](https://github.com/jsunyermias/bitturbulence/issues/111) | BEP 46 DHT Mutable Updates: auto-actualizar metainfo de un flow via DHT mutable items | 🔵 Baja |
| [#112](https://github.com/jsunyermias/bitturbulence/issues/112) | Predictive piece announce: anunciar pieza a DHT/peers antes de completar la verificación hash | 🔵 Baja |
| [#113](https://github.com/jsunyermias/bitturbulence/issues/113) | Piece extent affinity: solicitar bloques consecutivos al mismo peer para localidad de disco | 🔵 Baja |
| [#114](https://github.com/jsunyermias/bitturbulence/issues/114) | Etiquetas y categorías: agrupar flows por etiqueta y aplicar reglas (throttle, carpeta, ratio) | 🔵 Baja |
| [#115](https://github.com/jsunyermias/bitturbulence/issues/115) | Bandwidth scheduler: límites de subida/bajada diferentes por franja horaria | 🔵 Baja |
| [#117](https://github.com/jsunyermias/bitturbulence/issues/117) | Share mode: seedear piezas de otros flows para maximizar ratio sin descargar contenido útil | 🔵 Baja |
| [#119](https://github.com/jsunyermias/bitturbulence/issues/119) | Peer fingerprint configurable: peer ID con versión de cliente identificable opcionalmente | 🔵 Baja |
| [#136](https://github.com/jsunyermias/bitturbulence/issues/136) | Piece deadlines: deadline temporal por pieza para garantizar entrega antes de un timestamp (streaming preciso) | 🔵 Baja |
| [#137](https://github.com/jsunyermias/bitturbulence/issues/137) | Delayed Have batching: agrupar mensajes HavePiece en lotes para reducir overhead de protocolo | 🔵 Baja |
| [#120](https://github.com/jsunyermias/bitturbulence/issues/120) | lt_trackers extension: intercambiar lista de trackers activos entre peers conectados | 🔵 Baja |
| [#20](https://github.com/jsunyermias/bitturbulence/issues/20) | Limitación de velocidad: throttling global de subida/bajada | 🔵 Baja |
| [#7](https://github.com/jsunyermias/bitturbulence/issues/7) | Pool de conexiones de peers con límite global | 🔵 Baja |
| [#22](https://github.com/jsunyermias/bitturbulence/issues/22) | Vortex links: compartir flows sin `.bitflow` (`vortex://`) | 🔵 Baja |
| [#37](https://github.com/jsunyermias/bitturbulence/issues/37) | Web UI: panel de control con progreso en tiempo real via WebSocket | 🔵 Baja |
| [#25](https://github.com/jsunyermias/bitturbulence/issues/25) | Renombrar repositorio de quictorrent a bitturbulence | 🔵 Admin |
| [#34](https://github.com/jsunyermias/bitturbulence/issues/34) | Soporte Windows: compilación y CI en Windows | 🔵 Baja |
| [#35](https://github.com/jsunyermias/bitturbulence/issues/35) | Packaging: .deb, .rpm, Flatpak y Homebrew | 🔵 Baja |

## Descartados

Issues cerrados por ser innecesarios dado que bitturbulence es un protocolo propio sobre QUIC.

| # | Título | Razón |
|---|--------|-------|
| [#13](https://github.com/jsunyermias/bitturbulence/issues/13) | Cliente tracker HTTP/UDP estándar (compat. BitTorrent) | Retrocompat: bitturbulence usa su propio tracker QUIC |
| [#60](https://github.com/jsunyermias/bitturbulence/issues/60) | Keepalive adaptativo para móviles | Redundante: QUIC gestiona keepalives a nivel de transporte via `max_idle_timeout` |
| [#79](https://github.com/jsunyermias/bitturbulence/issues/79) | Importar .torrent estándar: convertir BitTorrent v1/v2 a .bitflow | Retrocompat: bitturbulence no es un cliente TCP BitTorrent |
| [#80](https://github.com/jsunyermias/bitturbulence/issues/80) | Web seeds BEP 19: descargar piezas via HTTP | Retrocompat: los seeds son peers QUIC, no HTTP endpoints |
| [#121](https://github.com/jsunyermias/bitturbulence/issues/121) | BEP 52 BitTorrent v2: leer metainfo v2 con SHA-256 | Retrocompat: dependía de #79, que se descarta |

## Cerrados recientemente

| # | Título | Commit |
|---|--------|--------|
| [#38](https://github.com/jsunyermias/bitturbulence/issues/38) | Filler: procesar mensajes Cancel para evitar servir bloques ya cancelados | 159307d |
| [#44](https://github.com/jsunyermias/bitturbulence/issues/44) | Logging estructurado: tracing spans con peer_id/addr/info_hash por conexión | 8555d15 |
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
