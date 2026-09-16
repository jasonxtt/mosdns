//! Synthetic CA/server certificate fixtures for the Slice1 `DoT` tests.
//!
//! Everything here is generated **in memory at test runtime** with the
//! test-only `rcgen` dev-dependency. No certificate, and in particular no
//! private key, is checked into the repository: the module contains no key
//! material and the keys it produces live only for the duration of a test
//! process. Nothing in this module is compiled into the library.
//!
//! ## Provenance and generation
//!
//! Every test generates its own fresh EC P-256 keys, so the material is
//! per-run and cannot be reused. The parameters match the reviewed Slice1
//! contract and are recorded here rather than in committed bytes:
//!
//! * Roots: self-signed, `IsCa::Ca(BasicConstraints::Constrained(0))`,
//!   `keyCertSign` + `cRLSign`, valid 2024-01-01 to 2036-01-01 UTC.
//!   Root A is the trusted anchor; root B is deliberately never trusted.
//! * Leaves: signed by their root, `CA:FALSE`, `digitalSignature`,
//!   `extendedKeyUsage = serverAuth`, and one `subjectAltName=DNS:<name>`.
//! * The expired leaf is generated with a validity window of
//!   2020-01-01 to 2021-01-01, so it is expired for any realistic run and the
//!   expiry case needs no clock control. The mismatch and unknown-issuer cases
//!   are time-independent.
//!
//! `rcgen` is declared as a dev-dependency only, so it is absent from the
//! library's normal dependency graph and cannot reach the production secure
//! modules. It is used here for the same reason the slice previously used
//! checked-in DER: deterministic, offline, non-secret test material.
//!
//! Root A is the only anchor in [`root_store_a`]; root B is only ever used to
//! build [`root_store_b`], which exists so a test can prove the unknown-issuer
//! case is about the anchor set and not an unparseable certificate.

#![allow(dead_code)]

use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, SanType, date_time_ymd,
};
use rustls::RootCertStore;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// One generated root authority.
struct RootAuthority {
    /// The root's self-signed certificate.
    params: CertificateParams,
    /// The root's private key.
    key: KeyPair,
    /// The DER-encoded root certificate, for trust-anchor construction.
    cert_der: CertificateDer<'static>,
}

impl RootAuthority {
    /// Generates a fresh self-signed root authority.
    fn generate(common_name: &str) -> Self {
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("generate a synthetic root key");
        let mut params = CertificateParams::default();
        params
            .distinguished_name
            .push(DnType::CommonName, common_name);
        params.not_before = date_time_ymd(2024, 1, 1);
        params.not_after = date_time_ymd(2036, 1, 1);
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let cert = params
            .self_signed(&key)
            .expect("self-sign the synthetic root");
        let cert_der = cert.der().clone();
        Self {
            params,
            key,
            cert_der,
        }
    }

    /// Issues a leaf for `dns_name` with an explicit validity window.
    fn issue_leaf(
        &self,
        dns_name: &str,
        not_before: (i32, u8, u8),
        not_after: (i32, u8, u8),
        serial: u64,
    ) -> ServerIdentityFixture {
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("generate a synthetic leaf key");
        let mut params = CertificateParams::default();
        params.distinguished_name.push(DnType::CommonName, dns_name);
        params.subject_alt_names = vec![SanType::DnsName(
            dns_name
                .try_into()
                .expect("synthetic SAN is a valid DNS name"),
        )];
        params.not_before = date_time_ymd(not_before.0, not_before.1, not_before.2);
        params.not_after = date_time_ymd(not_after.0, not_after.1, not_after.2);
        params.serial_number = Some(serial.into());
        params.is_ca = IsCa::ExplicitNoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];

        let issuer = Issuer::from_params(&self.params, &self.key);
        let cert = params
            .signed_by(&key, &issuer)
            .expect("sign the synthetic leaf");

        ServerIdentityFixture {
            cert: cert.der().clone(),
            key: PrivateKeyDer::try_from(key.serialize_der())
                .expect("the generated leaf key is valid PKCS#8"),
        }
    }
}

/// A generated leaf certificate together with the private key that matches it.
///
/// The key is held only for the lifetime of the test process and is never
/// written to disk or committed.
pub struct ServerIdentityFixture {
    /// The DER-encoded leaf certificate.
    pub cert: CertificateDer<'static>,
    /// The DER-encoded PKCS#8 private key matching [`Self::cert`].
    pub key: PrivateKeyDer<'static>,
}

/// The synthetic certificate set one test needs.
///
/// Generating this is deterministic in structure but not in key bytes, so each
/// test owns its own set and no test can depend on another's material.
pub struct FixtureSet {
    root_a: RootAuthority,
    root_b: RootAuthority,
    /// A trusted leaf valid for `dns.example` under root A.
    pub good: ServerIdentityFixture,
    /// A trusted leaf valid only for `other.example`, for the name-mismatch case.
    pub wrong_name: ServerIdentityFixture,
    /// A trusted leaf for `dns.example` whose validity window has already ended.
    pub expired: ServerIdentityFixture,
    /// A leaf for `dns.example` issued by the untrusted root B.
    pub unknown_issuer: ServerIdentityFixture,
}

impl FixtureSet {
    /// Generates a complete fresh fixture set.
    ///
    /// The keys are new on every call, which is what makes the material
    /// per-run rather than a committed constant.
    pub fn generate() -> Self {
        let root_a = RootAuthority::generate("mosdns-slice1-synthetic-root-a");
        let root_b = RootAuthority::generate("mosdns-slice1-synthetic-root-b");

        let good = root_a.issue_leaf("dns.example", (2024, 1, 1), (2036, 1, 1), 2001);
        let wrong_name = root_a.issue_leaf("other.example", (2024, 1, 1), (2036, 1, 1), 2002);
        // Expired for any realistic run, so no clock control is needed.
        let expired = root_a.issue_leaf("dns.example", (2020, 1, 1), (2021, 1, 1), 2003);
        let unknown_issuer = root_b.issue_leaf("dns.example", (2024, 1, 1), (2036, 1, 1), 2004);

        Self {
            root_a,
            root_b,
            good,
            wrong_name,
            expired,
            unknown_issuer,
        }
    }

    /// The trust-anchor store holding only root A.
    #[must_use]
    pub fn root_store_a(&self) -> RootCertStore {
        let mut roots = RootCertStore::empty();
        roots
            .add(self.root_a.cert_der.clone())
            .expect("synthetic root A parses as a trust anchor");
        roots
    }

    /// The trust-anchor store holding only root B.
    #[must_use]
    pub fn root_store_b(&self) -> RootCertStore {
        let mut roots = RootCertStore::empty();
        roots
            .add(self.root_b.cert_der.clone())
            .expect("synthetic root B parses as a trust anchor");
        roots
    }
}
