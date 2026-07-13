//! TLS server-certificate hot-swap reloadable subsystem.
//!
//! [`TlsSnapshotReloadable`] is the sole in-process writer of the served TLS
//! leaf certificate: on `apply`, it independently loads and verifies the
//! configured cert/key pair from disk and atomically swaps it into the
//! shipped [`crate::server_cert_resolver::ControllerServerCertResolver`]
//! (an `arc_swap`-backed [`rustls::server::ResolvesServerCert`]), so the next
//! TLS handshake serves the new leaf without a process restart.
//!
//! # No validate→apply stash
//!
//! `validate()` performs a full load-and-build of the candidate
//! [`rustls::sign::CertifiedKey`] purely as a proof-of-loadability check, then
//! **discards** the result — it returns only `Ok(())`/`Err`. `apply()` does
//! **not** consume anything validate produced; it independently re-loads and
//! re-verifies the pair from disk itself before swapping. The
//! [`Reloadable`] trait gives no ordering guarantee that the next call after
//! `validate()` is the matching `apply()`, so a stash consumed by a
//! mismatched later `apply()` would be a new bug class. The only state
//! shared across calls is `apply()`'s own pre-apply snapshot in `prior`,
//! stashed solely so `revert()` can restore it — the same sibling snapshot
//! idiom [`super::db_pool::DbPoolReloadable`] uses (validate compares, apply
//! independently (re)connects).
//!
//! # Ownership invariant (no CAS guard in `revert`)
//!
//! In external-cert deployments this reloadable is the resolver's single
//! writer by construction: automatic PKI-managed cert renewal is gated off
//! whenever an external cert is configured (`!has_external_tls_cert`), and
//! manual server-cert renewal via the admin API is rejected when the
//! controller is running with an externally managed certificate (closed by a
//! later hardening task). With no second writer able to race a swap,
//! `revert()`'s unconditional in-memory restore of `prior` is correct as-is
//! — it must not be guarded by a compare-and-swap.
//!
//! # No `tls.*` CA/trust-material wiring
//!
//! [`TlsConfig`] carries only the served leaf's `cert_path`/`key_path`,
//! `sans`, and `trust_domain` (restart-gated below) — no CA material. A
//! `tls.*` delta therefore never needs to touch
//! [`uptrakit_web_api::pki_utils`]-style CA/verifier wiring; only the leaf
//! resolver is in scope for this reloadable.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rootcause::prelude::*;
use rustls::sign::CertifiedKey;
use uptrakit_config_reload::config::TlsConfig;
use uptrakit_config_reload::defaults::WATCHDOG_HTTPS;
use uptrakit_config_reload::delta::RuntimeConfigDelta;
use uptrakit_config_reload::error::ConfigReloadError;
use uptrakit_config_reload::reloadable::Reloadable;

use crate::server_cert_resolver::ControllerServerCertResolver;

/// A [`Reloadable`] subsystem that hot-swaps the TLS server leaf certificate
/// served by [`ControllerServerCertResolver`].
///
/// See the module docs for the no-stash validate/apply split and the
/// ownership invariant that makes `revert()` safe without a CAS guard.
pub(crate) struct TlsSnapshotReloadable {
    /// TLS config the process booted with (trust_domain compare + change
    /// detection against `cert_path`/`key_path`).
    boot_tls: TlsConfig,
    /// The resolver whose served leaf this reloadable hot-swaps.
    resolver: Arc<ControllerServerCertResolver>,
    /// Pre-apply key stashed by `apply()`, restored by `revert()`. This is
    /// the ONLY cross-call state derived from `validate()` — never a
    /// prepared "new" key handed from `validate()`.
    prior: Mutex<Option<Arc<CertifiedKey>>>,
    /// The leaf `apply()` actually swapped into the resolver this cycle,
    /// used solely so `health_check()` can confirm the resolver still
    /// serves it. Cleared alongside `prior` whenever a cycle does not swap.
    applied: Mutex<Option<Arc<CertifiedKey>>>,
}

/// Load and build a [`CertifiedKey`] from the cert/key paths in `cfg`.
///
/// Used independently by both `validate()` (proof-of-loadability, result
/// discarded) and `apply()` (the value that is actually swapped in) — see
/// the module docs for why the two phases never share this result.
fn load_and_build(cfg: &TlsConfig) -> Result<Arc<CertifiedKey>, Report> {
    let bundle =
        crate::pki::load_external_cert(Path::new(&cfg.cert_path), Path::new(&cfg.key_path))?;
    let ck = crate::pki::build_certified_key(&bundle.cert_pem, &bundle.key_pem)?;
    // `build_certified_key` parses cert and key independently; it does NOT verify
    // that the private key matches the leaf certificate. Do that here so a
    // mid-rotation cert/key mismatch surfaces as an error at validate/apply time
    // rather than serving an unusable pair on the next handshake.
    ck.keys_match().map_err(|e| {
        report!(ConfigReloadError::Validate(format!(
            "tls cert/key pair mismatch: {e}"
        )))
    })?;
    Ok(Arc::new(ck))
}

/// Whether `new`'s cert/key paths differ from the paths this reloadable
/// booted with — the only trigger for a load+swap.
fn cert_paths_changed(boot_tls: &TlsConfig, new: &TlsConfig) -> bool {
    new.cert_path != boot_tls.cert_path || new.key_path != boot_tls.key_path
}

impl TlsSnapshotReloadable {
    /// Create a new `TlsSnapshotReloadable` bound to the resolver whose leaf
    /// it will hot-swap.
    pub(crate) fn new(boot_tls: TlsConfig, resolver: Arc<ControllerServerCertResolver>) -> Self {
        Self {
            boot_tls,
            resolver,
            prior: Mutex::new(None),
            applied: Mutex::new(None),
        }
    }
}

impl Reloadable for TlsSnapshotReloadable {
    type Config = TlsConfig;

    fn name(&self) -> &'static str {
        "tls_snapshot"
    }

    /// Validate that the incoming config is internally consistent and, if
    /// the cert/key paths changed, that the new pair is loadable.
    ///
    /// This performs a full load-and-build of the candidate `CertifiedKey`
    /// (surfacing a mid-rotation cert/key mismatch as a validate failure),
    /// then discards it — see the module docs on the no-stash split.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - [`TlsConfig::validate`] rejects the config (charset / both-or-neither
    ///   checks),
    /// - `trust_domain` changed (requires restart — mTLS identity is derived
    ///   from it at boot),
    /// - the new paths are empty while boot paths were set (removing an
    ///   external cert has no load path back to the internal PKI cert;
    ///   requires restart), or
    /// - the new cert/key pair cannot be loaded or fails to parse as a
    ///   matched pair.
    fn validate(&self, new: &TlsConfig) -> Result<(), Report> {
        new.validate()?;

        if new.trust_domain != self.boot_tls.trust_domain {
            bail!(ConfigReloadError::Validate(
                "tls.trust_domain change requires restart".into()
            ));
        }

        if !cert_paths_changed(&self.boot_tls, new) {
            return Ok(());
        }

        if new.cert_path.is_empty() && !self.boot_tls.cert_path.is_empty() {
            bail!(ConfigReloadError::Validate(
                "removing the external certificate requires restart".into()
            ));
        }

        // Proof-of-loadability only; the built key is intentionally dropped.
        let _ = load_and_build(new)?;
        Ok(())
    }

    /// Hot-swap the resolver's served leaf if the cert/key paths changed.
    ///
    /// Independently re-loads and re-verifies the pair from disk (a second,
    /// cheap disk read — never the value `validate()` built). If unchanged,
    /// this is a no-op.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::ApplyFailed`] if the re-load/re-verify
    /// fails (e.g. a mid-gap file change between `validate()` and `apply()`).
    /// No mutation happens before this point, so a failure here leaves the
    /// resolver untouched.
    async fn apply(&self, new: Arc<TlsConfig>) -> Result<(), Report> {
        if !cert_paths_changed(&self.boot_tls, &new) {
            // No swap this cycle — clear any stale state from a prior cycle
            // so revert()/health_check() never act on a leaf that no longer
            // reflects what apply() did just now.
            *self.prior.lock() = None;
            *self.applied.lock() = None;
            return Ok(());
        }

        let ck = load_and_build(&new).map_err(|e| {
            report!(ConfigReloadError::ApplyFailed {
                subsystem: "tls_snapshot".into(),
                message: e.to_string(),
            })
        })?;

        *self.prior.lock() = Some(self.resolver.current());
        *self.applied.lock() = Some(Arc::clone(&ck));
        self.resolver.swap(Arc::clone(&ck));
        tracing::info!(cert = %new.cert_path, key = %new.key_path, "tls server certificate hot-swapped");
        Ok(())
    }

    /// Restore the previously served leaf if `apply()` stashed one this
    /// cycle.
    ///
    /// Never re-reads disk — an `ArcSwap` swap is state restoration, not
    /// I/O. Safe to run unconditionally (no compare-and-swap) because this
    /// reloadable is the resolver's sole writer in external-cert
    /// deployments — see the module docs.
    ///
    /// # Errors
    ///
    /// Always returns `Ok(())` — if no snapshot exists there is nothing to
    /// revert and the resolver remains in its current state.
    async fn revert(&self) -> Result<(), Report> {
        if let Some(prior) = self.prior.lock().take() {
            self.resolver.swap(prior);
            tracing::info!("tls server certificate reverted");
        }
        Ok(())
    }

    /// If a swap happened this cycle, confirm the resolver now serves the
    /// leaf `apply()` swapped in.
    ///
    /// Proves the swap took; pair validity was already proven twice
    /// upstream (`validate()`'s full parse, `apply()`'s independent
    /// re-verify before the swap) — this performs no third validation.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigReloadError::HealthFailed`] if the resolver's served
    /// leaf does not match the leaf `apply()` swapped in.
    async fn health_check(&self) -> Result<(), Report> {
        let Some(applied) = self.applied.lock().clone() else {
            tracing::debug!("tls snapshot health check ok (no swap this cycle)");
            return Ok(());
        };

        let served = self.resolver.current();
        if served.cert.first() != applied.cert.first() {
            bail!(ConfigReloadError::HealthFailed {
                subsystem: "tls_snapshot".into(),
                message: "resolver leaf does not match the certificate applied this cycle".into(),
            });
        }
        tracing::debug!("tls snapshot health check ok");
        Ok(())
    }

    fn rollback_window(&self) -> Duration {
        WATCHDOG_HTTPS
    }
}

uptrakit_config_reload::reloadable_erased_impl!(TlsSnapshotReloadable, RuntimeConfigDelta::Tls);

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[expect(
        clippy::let_underscore_must_use,
        reason = "test code: idiomatic test patterns — discarding crypto provider init results"
    )]
    fn install_crypto_provider() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }

    /// Write a self-signed cert/key pair for `name` to unique temp files and
    /// return `(cert_path, key_path)`.
    fn write_cert_pair(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let key = rcgen::KeyPair::generate().expect("kp");
        let mut params = rcgen::CertificateParams::new(vec![name.into()]).expect("params");
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, name);
        let cert = params.self_signed(&key).expect("cert");

        let unique = format!("{}-{}-{}", name, std::process::id(), uuid::Uuid::new_v4());
        let cert_path = std::env::temp_dir().join(format!("{unique}.crt"));
        let key_path = std::env::temp_dir().join(format!("{unique}.key"));

        std::fs::File::create(&cert_path)
            .expect("create cert file")
            .write_all(cert.pem().as_bytes())
            .expect("write cert file");
        std::fs::File::create(&key_path)
            .expect("create key file")
            .write_all(key.serialize_pem().as_bytes())
            .expect("write key file");

        (cert_path, key_path)
    }

    fn tls_config_for(cert_path: &Path, key_path: &Path) -> TlsConfig {
        TlsConfig::new(
            cert_path.to_string_lossy().into_owned(),
            key_path.to_string_lossy().into_owned(),
        )
    }

    fn seeded_resolver(name: &str) -> (Arc<ControllerServerCertResolver>, Arc<CertifiedKey>) {
        install_crypto_provider();
        let key = rcgen::KeyPair::generate().expect("kp");
        let mut params = rcgen::CertificateParams::new(vec![name.into()]).expect("params");
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, name);
        let cert = params.self_signed(&key).expect("cert");
        let cert_der = rustls::pki_types::CertificateDer::from(cert.der().to_vec());
        let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(
            rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()),
        );
        let signing_key =
            rustls::crypto::aws_lc_rs::sign::any_supported_type(&key_der).expect("signing key");
        let ck = Arc::new(CertifiedKey::new(vec![cert_der], signing_key));
        (
            Arc::new(ControllerServerCertResolver::new(Arc::clone(&ck))),
            ck,
        )
    }

    #[test]
    fn tls_validate_rejects_empty_cert_path() {
        let cfg = TlsConfig::new("", "/etc/ssl/key.pem");
        let (resolver, _) = seeded_resolver("boot");
        let reloadable = TlsSnapshotReloadable::new(TlsConfig::default(), resolver);
        assert!(reloadable.validate(&cfg).is_err());
    }

    #[test]
    fn tls_validate_rejects_empty_key_path() {
        let cfg = TlsConfig::new("/etc/ssl/cert.pem", "");
        let (resolver, _) = seeded_resolver("boot");
        let reloadable = TlsSnapshotReloadable::new(TlsConfig::default(), resolver);
        assert!(reloadable.validate(&cfg).is_err());
    }

    #[test]
    fn tls_validate_rejects_trust_domain_change() {
        install_crypto_provider();
        let (cert_path, key_path) = write_cert_pair("a.local");
        let mut boot_tls = tls_config_for(&cert_path, &key_path);
        boot_tls.trust_domain = "boot.example".into();
        let (resolver, _) = seeded_resolver("boot");
        let reloadable = TlsSnapshotReloadable::new(boot_tls.clone(), resolver);

        let mut new = boot_tls;
        new.trust_domain = "changed.example".into();

        let err = reloadable.validate(&new).unwrap_err();
        assert!(err.to_string().contains("trust_domain"));
    }

    #[test]
    fn tls_validate_rejects_mismatched_pair() {
        install_crypto_provider();
        let (cert_a, _key_a) = write_cert_pair("a.local");
        let (_cert_b, key_b) = write_cert_pair("b.local");
        let (resolver, _) = seeded_resolver("boot");
        let boot_tls = TlsConfig::default();
        let reloadable = TlsSnapshotReloadable::new(boot_tls, resolver);

        // Mismatched pair: cert A with key B.
        let new = tls_config_for(&cert_a, &key_b);
        let err = reloadable.validate(&new).unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tls_apply_swaps_resolver() {
        install_crypto_provider();
        let (cert_a, key_a) = write_cert_pair("a.local");
        let (cert_b, key_b) = write_cert_pair("b.local");

        let boot_tls = tls_config_for(&cert_a, &key_a);
        let (resolver, key_ck_a) = seeded_resolver("a.local");
        let reloadable = TlsSnapshotReloadable::new(boot_tls, resolver.clone());

        let new_b = Arc::new(tls_config_for(&cert_b, &key_b));

        // validate() succeeds and does not mutate resolver state.
        reloadable.validate(&new_b).unwrap();
        assert_eq!(
            resolver.current().cert.first(),
            key_ck_a.cert.first(),
            "validate() must not mutate the resolver"
        );

        // apply() is called with a fresh TlsConfig, never anything validate produced.
        reloadable.apply(Arc::clone(&new_b)).await.unwrap();

        let after_apply = resolver.current();
        assert_ne!(
            after_apply.cert.first(),
            key_ck_a.cert.first(),
            "apply() must swap in a different leaf"
        );

        // Independently build B's expected leaf to compare DER bytes.
        let expected_b = load_and_build(&new_b).unwrap();
        assert_eq!(
            after_apply.cert.first(),
            expected_b.cert.first(),
            "resolver must serve B's leaf after apply()"
        );

        reloadable.revert().await.unwrap();
        let after_revert = resolver.current();
        assert_eq!(
            after_revert.cert.first(),
            key_ck_a.cert.first(),
            "revert() must restore A's leaf"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tls_apply_fails_cleanly_on_mid_gap_change() {
        install_crypto_provider();
        let (cert_a, key_a) = write_cert_pair("a.local");
        let (cert_b, key_b) = write_cert_pair("b.local");
        let (_cert_c, key_c) = write_cert_pair("c.local");

        let boot_tls = tls_config_for(&cert_a, &key_a);
        let (resolver, key_ck_a) = seeded_resolver("a.local");
        let reloadable = TlsSnapshotReloadable::new(boot_tls, resolver.clone());

        let new_b = tls_config_for(&cert_b, &key_b);

        // validate() succeeds against B's matched pair.
        reloadable.validate(&new_b).unwrap();

        // Simulate a mid-rotation write race: overwrite B's key file with a
        // third, non-matching key before apply() runs.
        std::fs::copy(&key_c, &key_b).expect("overwrite key file");

        let err = reloadable.apply(Arc::new(new_b)).await.unwrap_err();
        assert!(!err.to_string().is_empty());

        // No partial swap occurred; resolver still serves A's leaf.
        let served = resolver.current();
        assert_eq!(
            served.cert.first(),
            key_ck_a.cert.first(),
            "resolver must be unchanged after a failed apply()"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tls_validate_unchanged_paths_is_noop() {
        install_crypto_provider();
        let (cert_a, key_a) = write_cert_pair("a.local");
        let boot_tls = tls_config_for(&cert_a, &key_a);
        let (resolver, key_ck_a) = seeded_resolver("a.local");
        let reloadable = TlsSnapshotReloadable::new(boot_tls.clone(), resolver.clone());

        reloadable.validate(&boot_tls).unwrap();
        reloadable.apply(Arc::new(boot_tls)).await.unwrap();

        let served = resolver.current();
        assert_eq!(
            served.cert.first(),
            key_ck_a.cert.first(),
            "resolver must be untouched when paths are unchanged"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tls_health_check_detects_wrong_leaf() {
        install_crypto_provider();
        let (cert_a, key_a) = write_cert_pair("a.local");
        let (cert_b, key_b) = write_cert_pair("b.local");

        let boot_tls = tls_config_for(&cert_a, &key_a);
        let (resolver, _key_ck_a) = seeded_resolver("a.local");
        let reloadable = TlsSnapshotReloadable::new(boot_tls, resolver.clone());

        let new_b = Arc::new(tls_config_for(&cert_b, &key_b));
        reloadable.apply(Arc::clone(&new_b)).await.unwrap();
        reloadable.health_check().await.unwrap();

        // Simulate a failed/overwritten swap: something else swaps in a
        // third leaf the reloadable never recorded as `applied`.
        let (_resolver_c, key_ck_c) = seeded_resolver("c.local");
        resolver.swap(key_ck_c);

        let err = reloadable.health_check().await.unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tls_health_check_ok_after_apply() {
        install_crypto_provider();
        let (cert_a, key_a) = write_cert_pair("a.local");
        let (cert_b, key_b) = write_cert_pair("b.local");

        let boot_tls = tls_config_for(&cert_a, &key_a);
        let (resolver, _key_ck_a) = seeded_resolver("a.local");
        let reloadable = TlsSnapshotReloadable::new(boot_tls, resolver);

        let new_b = Arc::new(tls_config_for(&cert_b, &key_b));
        reloadable.apply(Arc::clone(&new_b)).await.unwrap();

        reloadable.health_check().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tls_health_check_ok_no_swap() {
        install_crypto_provider();
        let (cert_a, key_a) = write_cert_pair("a.local");
        let boot_tls = tls_config_for(&cert_a, &key_a);
        let (resolver, _key_ck_a) = seeded_resolver("a.local");
        let reloadable = TlsSnapshotReloadable::new(boot_tls.clone(), resolver);

        // Unchanged paths: apply() is a no-op.
        reloadable.apply(Arc::new(boot_tls)).await.unwrap();

        reloadable.health_check().await.unwrap();
    }

    #[test]
    fn tls_validate_rejects_removing_external_cert() {
        install_crypto_provider();
        let (cert_a, key_a) = write_cert_pair("a.local");
        let boot_tls = tls_config_for(&cert_a, &key_a);
        let (resolver, _) = seeded_resolver("a.local");
        let reloadable = TlsSnapshotReloadable::new(boot_tls, resolver);

        let new = TlsConfig::default(); // empty cert_path/key_path
        let err = reloadable.validate(&new).unwrap_err();
        assert!(err.to_string().contains("restart"));
    }
}
