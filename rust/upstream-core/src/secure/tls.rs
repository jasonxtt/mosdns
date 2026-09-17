//! Explicit TLS authentication policy for secure upstreams (Phase 4 Slice0).
//!
//! This module freezes how a caller states TLS trust before any socket exists.
//! It deliberately contains no I/O: it neither builds a rustls `ClientConfig`
//! nor opens a connection or performs a handshake. A later secure-transport
//! slice builds its client configuration from this policy on the caller's
//! host-owned runtime.
//!
//! Exactly two modes exist:
//!
//! * [`TlsPolicy::verified`] preserves caller-supplied trust roots. The rustls
//!   webpki verifier then checks the certificate chain, the service identity
//!   and the validity window, and continues to verify the TLS handshake
//!   signature with the selected provider. Root discovery and platform trust
//!   loading remain host-layer work, so the policy never substitutes an OS
//!   store for the explicit roots.
//! * [`TlsPolicy::insecure_skip_verify`] is the explicit opt-in equivalent of
//!   the existing `insecure_skip_verify` setting. It skips chain/name/time
//!   checks only because the caller asked for it. It is never selected
//!   automatically: no constructor, setter, or error path downgrades a
//!   verified policy after an authentication failure. A later handshake slice
//!   still runs the provider's TLS1.2/TLS1.3 handshake signature verification
//!   for this mode.
//!
//! The policy enables none of the optional TLS features by default: no 0-RTT
//! early data, session resumption, client certificates (mTLS), custom cipher
//! suites, or custom protocol versions. Later I/O slices must keep those off;
//! enabling one needs a separate reviewed requirement.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};

use super::endpoint::ServerIdentity;
use super::error::{CertificateRejection, SecureError, TlsConfigError, TlsHandshakeFailure};

/// A frozen TLS authentication policy.
///
/// The mode is private and immutable; calling a constructor is the only way to
/// choose it. See the module documentation for the verified/insecure contract.
#[derive(Clone, Debug)]
pub struct TlsPolicy {
    mode: TlsMode,
}

#[derive(Clone, Debug)]
enum TlsMode {
    Verified(RootCertStore),
    InsecureSkipVerify,
}

impl TlsPolicy {
    /// Creates a verified policy from explicit trust roots.
    ///
    /// The caller's [`RootCertStore`] is moved into the policy unchanged, so a
    /// later I/O slice verifies against exactly these anchors and no other.
    ///
    /// # Errors
    ///
    /// Returns [`SecureError::TlsConfig`] carrying
    /// [`TlsConfigError::EmptyRootStore`] when `roots` has no trust anchors.
    /// The rejection happens before any socket or resolver work and never
    /// degrades to insecure verification.
    pub fn verified(roots: RootCertStore) -> Result<Self, SecureError> {
        if roots.is_empty() {
            return Err(SecureError::TlsConfig(TlsConfigError::EmptyRootStore));
        }
        Ok(Self {
            mode: TlsMode::Verified(roots),
        })
    }

    /// Creates the explicit opt-in policy that skips certificate chain, name
    /// and validity checks.
    ///
    /// This mirrors the existing `insecure_skip_verify` setting and is only
    /// reached by calling this constructor. An authentication failure on a
    /// [`Self::verified`] policy is terminal and never selects this mode.
    #[must_use]
    pub const fn insecure_skip_verify() -> Self {
        Self {
            mode: TlsMode::InsecureSkipVerify,
        }
    }

    /// Whether this policy skips certificate chain/name/time verification.
    #[must_use]
    pub const fn is_insecure_skip_verify(&self) -> bool {
        matches!(self.mode, TlsMode::InsecureSkipVerify)
    }

    /// The caller-supplied trust roots, or `None` for the insecure mode.
    #[must_use]
    pub const fn roots(&self) -> Option<&RootCertStore> {
        match &self.mode {
            TlsMode::Verified(roots) => Some(roots),
            TlsMode::InsecureSkipVerify => None,
        }
    }

    /// The number of preserved trust anchors, or `None` for the insecure mode.
    #[must_use]
    pub fn root_count(&self) -> Option<usize> {
        self.roots().map(RootCertStore::len)
    }

    /// Builds a rustls client configuration offering exactly `alpn`.
    ///
    /// This is the same configuration [`Self::client_config`] builds, with the
    /// ALPN list set to the caller's protocols. It exists so a transport can
    /// offer precisely the protocols it implements without mutating a shared
    /// `Arc<ClientConfig>` or widening the verification policy in any way.
    ///
    /// # Errors
    ///
    /// Returns [`SecureError::TlsConfig`] with [`TlsConfigError::Provider`] only
    /// if the selected provider supports none of the safe default protocol
    /// versions.
    pub(crate) fn client_config_with_alpn(
        &self,
        alpn: &[&[u8]],
    ) -> Result<ClientConfig, SecureError> {
        let config = self.client_config()?;
        let mut config = ClientConfig::clone(&config);
        config.alpn_protocols = alpn.iter().copied().map(<[u8]>::to_vec).collect();
        Ok(config)
    }

    /// Builds the rustls client configuration for this policy.
    ///
    /// The configuration is built per exchange from the frozen policy, so a
    /// verified policy always verifies against exactly the caller's anchors and
    /// an insecure policy always uses the explicit no-verification verifier.
    /// No constructor, setter, or error path can convert one into the other, so
    /// an authentication failure can never downgrade to insecure verification.
    ///
    /// The `ring` provider is selected explicitly and the safe default protocol
    /// versions are used, which keeps the provider's TLS1.2/TLS1.3 handshake
    /// signature verification active. Client authentication, early data (0-RTT)
    /// and session resumption are all disabled, matching the module contract.
    ///
    /// # Errors
    ///
    /// Returns [`SecureError::TlsConfig`] with [`TlsConfigError::Provider`] only
    /// if the selected provider supports none of the safe default protocol
    /// versions, which cannot happen for the reviewed `ring` feature set.
    pub(crate) fn client_config(&self) -> Result<Arc<ClientConfig>, SecureError> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = ClientConfig::builder_with_provider(Arc::clone(&provider))
            .with_safe_default_protocol_versions()
            .map_err(|_| SecureError::TlsConfig(TlsConfigError::Provider))?;

        let mut config = match &self.mode {
            TlsMode::Verified(roots) => builder
                .with_root_certificates(roots.clone())
                .with_no_client_auth(),
            TlsMode::InsecureSkipVerify => builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(InsecureVerifier {
                    algorithms: provider.signature_verification_algorithms,
                }))
                .with_no_client_auth(),
        };

        // No ALPN is advertised for DoT: RFC 7858 defines no ALPN protocol for
        // DNS-over-TLS, and offering one could invite a peer to select a
        // protocol this primitive does not speak.
        config.alpn_protocols.clear();
        // No 0-RTT early data and no session resumption in this foundation.
        config.enable_early_data = false;
        config.resumption = rustls::client::Resumption::disabled();
        Ok(Arc::new(config))
    }
}

/// The server name a handshake is authenticated against.
///
/// A DNS identity becomes a DNS `ServerName`, which is also what produces the
/// SNI extension. An IP identity becomes an IP `ServerName`, which performs no
/// SNI and validates the certificate's IP SANs instead. This is derived from
/// the service identity and never from the numeric dial address.
///
/// # Errors
///
/// Returns [`SecureError::InvalidIdentity`] when the identity cannot be
/// represented as a TLS server name.
pub(crate) fn server_name_for(
    identity: &ServerIdentity,
) -> Result<ServerName<'static>, SecureError> {
    match identity.dns_name() {
        Some(name) => ServerName::try_from(name)
            .map(|name| name.to_owned())
            .map_err(|_| SecureError::InvalidIdentity(super::error::IdentityError::Malformed)),
        None => identity
            .ip()
            .map(ServerName::from)
            .ok_or(SecureError::InvalidIdentity(
                super::error::IdentityError::Malformed,
            )),
    }
}

/// The explicit opt-in verifier used by [`TlsPolicy::insecure_skip_verify`].
///
/// It skips exactly the chain, service-name and validity-window checks. It
/// deliberately does **not** skip the handshake signature checks: those are
/// delegated to the selected provider's webpki helpers, so the peer must still
/// prove possession of the private key for the certificate it presented. This
/// mirrors the existing `insecure_skip_verify` configuration intent while
/// keeping the cryptographic handshake proofs enforced.
#[derive(Debug)]
struct InsecureVerifier {
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for InsecureVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// Classifies a rustls handshake error into the typed secure-failure vocabulary.
///
/// Only structured library outcomes are inspected; the returned value never
/// carries the peer's certificate, key material, or a raw error string.
pub(crate) fn classify_handshake_error(error: &rustls::Error) -> TlsHandshakeFailure {
    use rustls::Error as Tls;

    match error {
        Tls::InvalidCertificate(reason) => {
            TlsHandshakeFailure::Certificate(classify_certificate_error(reason))
        }
        Tls::InvalidMessage(_) => TlsHandshakeFailure::Protocol,
        Tls::AlertReceived(_) => TlsHandshakeFailure::Alert,
        Tls::NoCertificatesPresented
        | Tls::UnsupportedNameType
        | Tls::InappropriateMessage { .. }
        | Tls::InappropriateHandshakeMessage { .. }
        | Tls::PeerIncompatible(_)
        | Tls::PeerMisbehaved(_)
        | Tls::InvalidEncryptedClientHello(_) => TlsHandshakeFailure::Protocol,
        _ => TlsHandshakeFailure::Other,
    }
}

/// Classifies a certificate-verification outcome without echoing certificate
/// data.
fn classify_certificate_error(error: &rustls::CertificateError) -> CertificateRejection {
    use rustls::CertificateError as Certificate;

    match error {
        Certificate::Expired | Certificate::ExpiredContext { .. } => CertificateRejection::Expired,
        Certificate::NotValidYet | Certificate::NotValidYetContext { .. } => {
            CertificateRejection::NotValidYet
        }
        Certificate::NotValidForName | Certificate::NotValidForNameContext { .. } => {
            CertificateRejection::NotValidForName
        }
        Certificate::UnknownIssuer => CertificateRejection::UnknownIssuer,
        Certificate::BadSignature => CertificateRejection::BadSignature,
        Certificate::BadEncoding => CertificateRejection::BadEncoding,
        _ => CertificateRejection::Other,
    }
}
