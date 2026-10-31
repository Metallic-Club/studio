//! The server's self-signed certificate and the client's pinned verifier.
//!
//! There is no CA and no DNS: the client is told the SHA-256 of the server's certificate (DER) out of band and
//! accepts exactly that certificate, inside its validity window.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::ParsedCertificate;
use rustls::{CertificateError, DigitallySignedStruct, RootCertStore, SignatureScheme};
use sha2::{Digest, Sha256};

use crate::NetError;

/// Name in the certificate and the TLS server name the client sends.
pub(crate) const CERT_NAME: &str = "metallic";
/// How long a fresh certificate is valid.
pub(crate) const CERT_LIFETIME: Duration = Duration::from_secs(13 * 24 * 60 * 60);
/// Backdating of `not_before`, for clocks that run a little behind.
const CLOCK_SKEW: Duration = Duration::from_secs(60 * 60);

/// A certificate and its private key. Not `Debug`: the key never reaches a log.
pub(crate) struct ServerIdentity {
    pub(crate) cert: CertificateDer<'static>,
    pub(crate) key: PrivateKeyDer<'static>,
    pub(crate) sha256: [u8; 32],
}

/// The crypto provider every config uses (ring; no aws-lc).
pub(crate) fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

pub(crate) fn sha256(der: &[u8]) -> [u8; 32] {
    Sha256::digest(der).into()
}

/// A fresh ECDSA P-256 self-signed certificate, valid for [`CERT_LIFETIME`] from now.
pub(crate) fn self_signed() -> Result<ServerIdentity, NetError> {
    let tls = |e: rcgen::Error| NetError::Tls(e.to_string());
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(tls)?;
    let mut params = rcgen::CertificateParams::new(vec![CERT_NAME.to_string()]).map_err(tls)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| NetError::Tls(e.to_string()))?;
    let at = |t: Duration| {
        let secs = i64::try_from(t.as_secs()).map_err(|e| NetError::Tls(e.to_string()))?;
        time::OffsetDateTime::from_unix_timestamp(secs).map_err(|e| NetError::Tls(e.to_string()))
    };
    params.not_before = at(now.saturating_sub(CLOCK_SKEW))?;
    params.not_after = at(now + CERT_LIFETIME)?;
    let cert = params.self_signed(&key).map_err(tls)?;
    let cert = cert.der().clone();
    Ok(ServerIdentity {
        sha256: sha256(&cert),
        cert,
        key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
    })
}

/// Accepts exactly the certificate whose SHA-256 is `pin`, when it is self-signed and within its validity window.
#[derive(Debug)]
pub(crate) struct PinnedVerifier {
    pin: [u8; 32],
    provider: Arc<CryptoProvider>,
}

impl PinnedVerifier {
    pub(crate) fn new(pin: [u8; 32], provider: Arc<CryptoProvider>) -> PinnedVerifier {
        PinnedVerifier { pin, provider }
    }
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if sha256(end_entity) != self.pin {
            return Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ));
        }
        // The pinned certificate is its own trust anchor: this checks its signature and validity window.
        let mut roots = RootCertStore::empty();
        roots.add(end_entity.clone().into_owned())?;
        let parsed = ParsedCertificate::try_from(end_entity)?;
        rustls::client::verify_server_cert_signed_by_trust_anchor(
            &parsed,
            &roots,
            &[],
            now,
            self.provider.signature_verification_algorithms.all,
        )?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verify(
        pin: [u8; 32],
        cert: &CertificateDer<'_>,
        at: Duration,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let name =
            ServerName::try_from(CERT_NAME).map_err(|e| rustls::Error::General(e.to_string()))?;
        PinnedVerifier::new(pin, provider()).verify_server_cert(
            cert,
            &[],
            &name,
            &[],
            UnixTime::since_unix_epoch(at),
        )
    }

    #[test]
    fn accepts_only_the_pinned_certificate_while_valid() {
        let id = self_signed().unwrap();
        let other = self_signed().unwrap();
        assert_ne!(id.sha256, other.sha256);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        assert!(verify(id.sha256, &id.cert, now).is_ok());
        assert!(verify(other.sha256, &id.cert, now).is_err(), "wrong pin");
        let day = Duration::from_secs(24 * 60 * 60);
        assert!(verify(id.sha256, &id.cert, now + 12 * day).is_ok());
        assert!(
            verify(id.sha256, &id.cert, now + 14 * day).is_err(),
            "expired"
        );
        assert!(
            verify(id.sha256, &id.cert, now - day).is_err(),
            "not yet valid"
        );
    }
}
