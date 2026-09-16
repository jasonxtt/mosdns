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

use rustls::RootCertStore;

use super::error::{SecureError, TlsConfigError};

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
}
