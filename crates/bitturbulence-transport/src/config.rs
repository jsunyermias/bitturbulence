use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use quinn::{ClientConfig, ServerConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, UnixTime};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::error::{Result, TransportError};

/// ALPN protocol id para bitturbulence.
/// Identifica el protocolo en el handshake TLS — los peers que no hablen
/// este protocolo serán rechazados automáticamente.
pub const ALPN_BITTURBULENCE: &[u8] = b"bitturbulence/1";

/// Almacén de fingerprints SHA-256 para TOFU (Trust-On-First-Use).
///
/// La primera vez que se conecta a un peer, se guarda el fingerprint de su
/// certificado. En conexiones posteriores, si el fingerprint cambia, la
/// conexión es rechazada para prevenir ataques MitM.
///
/// El almacén es barato de clonar (Arc interno) y puede compartirse entre
/// el `QuicEndpoint` y cualquier código que necesite consultar los peers
/// de confianza.
#[derive(Clone, Debug, Default)]
pub struct TofuStore {
    inner: Arc<Mutex<HashMap<SocketAddr, [u8; 32]>>>,
}

impl TofuStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Devuelve el fingerprint almacenado para una dirección, si existe.
    pub fn get(&self, addr: &SocketAddr) -> Option<[u8; 32]> {
        self.inner.lock().expect("TOFU store mutex poisoned").get(addr).copied()
    }
}

/// Genera un certificado TLS self-signed para un peer.
///
/// En BitTorrent sobre QUIC, la identidad del peer NO se deriva del cert
/// (a diferencia de mTLS clásico). El cert solo sirve para establecer el
/// canal cifrado; la autenticación real es a nivel de protocolo BT
/// (handshake con peer_id e info_hash).
pub fn generate_self_signed_cert() -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
    let cert = rcgen::generate_simple_self_signed(vec!["bitturbulence".into()])
        .map_err(|e| TransportError::CertGen(e.to_string()))?;

    let cert_der = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        cert.key_pair.serialize_der(),
    ));

    Ok((cert_der, key_der))
}

/// Configuración TLS para el lado servidor (acepta cualquier cliente).
pub fn server_config() -> Result<ServerConfig> {
    let (cert, key) = generate_self_signed_cert()?;

    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|e| TransportError::Tls(e.to_string()))?;

    tls.alpn_protocols = vec![ALPN_BITTURBULENCE.to_vec()];

    let transport = default_transport_config();
    let mut cfg = ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(tls)
            .map_err(|e| TransportError::Tls(e.to_string()))?,
    ));
    cfg.transport_config(Arc::new(transport));
    Ok(cfg)
}

/// Configuración TLS para el lado cliente con TOFU para un peer específico.
///
/// Cada conexión saliente recibe su propio `TofuVerifier` que comprueba
/// (y almacena) el fingerprint SHA-256 del certificado del peer. Si el
/// fingerprint cambia entre conexiones, se rechaza con error.
pub fn client_config_for_peer(peer_addr: SocketAddr, store: TofuStore) -> Result<ClientConfig> {
    let verifier = TofuVerifier { peer_addr, store };

    let mut tls = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();

    tls.alpn_protocols = vec![ALPN_BITTURBULENCE.to_vec()];

    let transport = default_transport_config();
    let mut cfg = ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls)
            .map_err(|e| TransportError::Tls(e.to_string()))?,
    ));
    cfg.transport_config(Arc::new(transport));
    Ok(cfg)
}

fn default_transport_config() -> quinn::TransportConfig {
    let mut t = quinn::TransportConfig::default();
    // Streams bidireccionales concurrentes máximos por conexión.
    // Cada pieza en vuelo usa un stream; 256 es conservador pero suficiente
    // para empezar. Se puede aumentar después de benchmarks.
    t.max_concurrent_bidi_streams(256u32.into());
    // Keep-alive cada 10s para mantener la conexión a través de NATs.
    t.keep_alive_interval(Some(std::time::Duration::from_secs(10)));
    // Timeout de idle: 30s sin actividad cierra la conexión.
    t.max_idle_timeout(Some(
        quinn::VarInt::from_u32(30_000).into(),
    ));
    t
}

fn fmt_fingerprint(f: &[u8; 32]) -> String {
    f.iter().map(|b| format!("{b:02x}")).collect()
}

/// Verificador TOFU: acepta el certificado la primera vez y comprueba
/// en conexiones posteriores que no ha cambiado.
///
/// Adicionalmente verifica la firma del handshake TLS (que el peer posee
/// la clave privada correspondiente al certificado presentado).
#[derive(Debug)]
struct TofuVerifier {
    peer_addr: SocketAddr,
    store:     TofuStore,
}

impl ServerCertVerifier for TofuVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let fingerprint: [u8; 32] = Sha256::digest(end_entity.as_ref()).into();

        let mut store = self.store.inner.lock()
            .expect("TOFU store mutex poisoned");

        match store.entry(self.peer_addr) {
            std::collections::hash_map::Entry::Vacant(e) => {
                info!(
                    peer  = %self.peer_addr,
                    fp    = %fmt_fingerprint(&fingerprint),
                    "TOFU: nuevo peer — fingerprint almacenado"
                );
                e.insert(fingerprint);
                Ok(ServerCertVerified::assertion())
            }
            std::collections::hash_map::Entry::Occupied(e) => {
                if e.get() == &fingerprint {
                    Ok(ServerCertVerified::assertion())
                } else {
                    warn!(
                        peer     = %self.peer_addr,
                        stored   = %fmt_fingerprint(e.get()),
                        received = %fmt_fingerprint(&fingerprint),
                        "TOFU: fingerprint cambiado — posible ataque MitM, conexión rechazada"
                    );
                    Err(rustls::Error::General(format!(
                        "TOFU violation: cert fingerprint changed for {}",
                        self.peer_addr
                    )))
                }
            }
        }
    }

    /// Verifica que el peer firmó el handshake con la clave privada del cert.
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    /// Verifica que el peer firmó el handshake con la clave privada del cert.
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
