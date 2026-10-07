//! Optionele TLS voor de IRC-verbinding (rustls + ring, webpki-roots). Zonder TLS gaat een SASL-wachtwoord in leesbare tekst over de lijn.

use std::io;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use tokio_rustls::rustls::crypto::{ring, verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use tokio_rustls::TlsConnector;

/// Eén type voor platte en TLS-verbindingen.
pub trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncStream for T {}
pub type BoxedStream = Box<dyn AsyncStream>;

/// Instellingen uit de omgeving: `IRC_USE_TLS` (standaard aan voor poort 6697) en `IRC_TLS_VERIFY` (standaard aan).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlsSettings {
    pub enabled: bool,
    pub verify: bool,
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "ja" => Some(true),
        "0" | "false" | "no" | "off" | "nee" => Some(false),
        _ => None,
    }
}

impl TlsSettings {
    pub fn from_env(port: u16) -> Self {
        let enabled = std::env::var("IRC_USE_TLS").ok().and_then(|v| parse_bool(&v)).unwrap_or(port == 6697);
        let verify = std::env::var("IRC_TLS_VERIFY").ok().and_then(|v| parse_bool(&v)).unwrap_or(true);
        Self { enabled, verify }
    }
}

/// Accepteert elk certificaat (alleen voor `IRC_TLS_VERIFY=false`, bijv. zelfondertekende netwerken). Versleuteld, maar niet geauthenticeerd.
#[derive(Debug)]
struct NoVerifier(Arc<CryptoProvider>);

impl ServerCertVerifier for NoVerifier {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, tokio_rustls::rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn client_config(verify: bool) -> io::Result<ClientConfig> {
    let provider = Arc::new(ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?;
    Ok(if verify {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        builder.with_root_certificates(roots).with_no_client_auth()
    } else {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerifier(provider)))
            .with_no_client_auth()
    })
}

/// Maakt de verbinding, met of zonder TLS.
pub async fn connect(server: &str, port: u16, tls: TlsSettings) -> io::Result<BoxedStream> {
    let tcp = TcpStream::connect((server, port)).await?;
    if !tls.enabled {
        return Ok(Box::new(tcp));
    }
    let connector = TlsConnector::from(Arc::new(client_config(tls.verify)?));
    let name = ServerName::try_from(server.to_string()).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    Ok(Box::new(connector.connect(name, tcp).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio_rustls::rustls::pki_types::PrivateKeyDer;
    use tokio_rustls::rustls::ServerConfig;
    use tokio_rustls::TlsAcceptor;

    #[test]
    fn env_defaults_and_parsing() {
        std::env::remove_var("IRC_USE_TLS");
        std::env::remove_var("IRC_TLS_VERIFY");
        assert_eq!(TlsSettings::from_env(6697), TlsSettings { enabled: true, verify: true });
        assert_eq!(TlsSettings::from_env(6667), TlsSettings { enabled: false, verify: true });
        assert_eq!(parse_bool("FALSE"), Some(false));
        assert_eq!(parse_bool("ja"), Some(true));
        assert_eq!(parse_bool("misschien"), None);
    }

    /// Verbinden met een TLS-server met zelfondertekend certificaat: geweigerd met verificatie, aangenomen zonder.
    #[tokio::test]
    async fn tls_handshake_with_and_without_verification() {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_der = CertificateDer::from(cert.cert.der().to_vec());
        let key_der = PrivateKeyDer::try_from(cert.key_pair.serialize_der()).unwrap();
        let provider = Arc::new(ring::default_provider());
        let server_cfg = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(server_cfg));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((sock, _)) = listener.accept().await else { break };
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    if let Ok(mut tls) = acceptor.accept(sock).await {
                        let mut buf = [0u8; 4];
                        if tls.read_exact(&mut buf).await.is_ok() {
                            let _ = tls.write_all(b"pong").await;
                        }
                    }
                });
            }
        });

        // Met verificatie wordt het zelfondertekende certificaat geweigerd
        assert!(connect("localhost", port, TlsSettings { enabled: true, verify: true }).await.is_err());

        // Zonder verificatie werkt de versleutelde verbinding
        let mut stream = connect("localhost", port, TlsSettings { enabled: true, verify: false }).await.unwrap();
        stream.write_all(b"ping").await.unwrap();
        let mut reply = [0u8; 4];
        stream.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"pong");
    }
}
