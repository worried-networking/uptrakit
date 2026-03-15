//! Phase 6: Settings reconciliation.

use std::fmt;
use std::net::SocketAddr;

use ipnet::IpNet;
use rootcause::prelude::*;
#[cfg(feature = "nats")]
use uptrakit_web_api::MaskedUrl;
use uptrakit_web_api::SettingKey;
use uptrakit_web_api::settings::Settings;
use uptrakit_web_api::settings_store::RawSettingsExt;

use super::ReconciledSettings;
use crate::AppError;

/// Reconcile a nullable string setting: empty string <-> JSON null, returning
/// `Some(value)` for non-empty values and `None` for empty/null.
async fn reconcile_nullable_string(
    db: &sea_orm::DatabaseConnection,
    key: SettingKey,
    raw: &uptrakit_web_api::settings_store::RawSettings,
    cli_value: Option<String>,
    force: bool,
) -> crate::Result<Option<String>> {
    let value = crate::reconcile::reconcile_setting(crate::reconcile::ReconcileParams {
        db,
        key,
        raw,
        cli_value,
        default_value: String::new(),
        force,
        convert: crate::reconcile::JsonConvert {
            to_json: |v| {
                if v.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::json!(v)
                }
            },
            from_json: |v| {
                if v.is_null() {
                    Some(String::new())
                } else {
                    v.as_str().map(String::from)
                }
            },
        },
    })
    .await
    .context(AppError::Settings)?;
    Ok(if value.is_empty() { None } else { Some(value) })
}

/// Reconcile all DB-managed global settings with CLI values and update the
/// in-memory [`Settings`] object.
pub(crate) async fn reconcile_all_settings(
    db: &sea_orm::DatabaseConnection,
    args: &crate::cli::Args,
    settings: &Settings,
    global_raw: &uptrakit_web_api::settings_store::RawSettings,
) -> crate::Result<ReconciledSettings> {
    let force = args.force_settings_override;

    // Network settings
    let trusted_proxies = reconcile_setting_vec::<IpNet>(crate::reconcile::ReconcileParams {
        db,
        key: SettingKey::TrustedProxies,
        raw: global_raw,
        cli_value: if args.trusted_proxies.is_empty() {
            None
        } else {
            Some(args.trusted_proxies.clone())
        },
        default_value: vec![],
        force,
        convert: crate::reconcile::JsonConvert {
            to_json: |v| serde_json::json!(v.iter().map(|n| n.to_string()).collect::<Vec<_>>()),
            from_json: |v| {
                v.as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str()?.parse::<IpNet>().ok())
                        .collect()
                })
            },
        },
    })
    .await
    .context(AppError::Settings)?;
    settings.set_trusted_proxies(trusted_proxies.clone()).await;
    for cidr in &trusted_proxies {
        warn_broad_trusted_proxy(cidr);
    }

    let real_ip_header = crate::reconcile::reconcile_setting(crate::reconcile::ReconcileParams {
        db,
        key: SettingKey::RealIpHeader,
        raw: global_raw,
        cli_value: args.real_ip_header.clone(),
        default_value: uptrakit_web_api::settings::DEFAULT_REAL_IP_HEADER.to_string(),
        force,
        convert: crate::reconcile::JsonConvert {
            to_json: |v| serde_json::json!(v),
            from_json: |v| v.as_str().map(String::from),
        },
    })
    .await
    .context(AppError::Settings)?;
    settings.set_real_ip_header(real_ip_header).await;

    let forwarded_cert_info_opt = reconcile_nullable_string(
        db,
        SettingKey::ForwardedClientCertInfoHeader,
        global_raw,
        args.forwarded_client_cert_info_header.clone(),
        force,
    )
    .await?;
    settings
        .set_forwarded_client_cert_info_header(forwarded_cert_info_opt.clone())
        .await;

    let forwarded_cert_pem_opt = reconcile_nullable_string(
        db,
        SettingKey::ForwardedClientCertPemHeader,
        global_raw,
        args.forwarded_client_cert_pem_header.clone(),
        force,
    )
    .await?;
    settings
        .set_forwarded_client_cert_pem_header(forwarded_cert_pem_opt.clone())
        .await;

    let pki_addr_opt = reconcile_nullable_string(
        db,
        SettingKey::PkiAddr,
        global_raw,
        args.pki_addr.clone(),
        force,
    )
    .await?;
    settings.set_pki_addr(pki_addr_opt.clone()).await;

    // Warn if cert headers are configured but no trusted proxies
    if (forwarded_cert_info_opt.is_some() || forwarded_cert_pem_opt.is_some())
        && trusted_proxies.is_empty()
    {
        tracing::warn!(
            "forwarded client cert header(s) configured but no --trusted-proxy set; \
             cert headers will be stripped from all requests"
        );
    }

    // SANs reconciliation: full-list semantics (not additive)
    //
    // 1. --san provided         -> standard 5-case reconcile (CLI is canonical)
    // 2. --san absent, DB has   -> use DB value (no auto-detection)
    // 3. --san absent, DB empty -> first start: auto-detect and save to DB
    let sans = if !args.sans.is_empty() {
        // Case 1: --san provided — standard reconcile
        reconcile_setting_vec::<String>(crate::reconcile::ReconcileParams {
            db,
            key: SettingKey::Sans,
            raw: global_raw,
            cli_value: Some(args.sans.clone()),
            default_value: vec![],
            force,
            convert: crate::reconcile::JsonConvert {
                to_json: |v| serde_json::json!(v),
                from_json: |v| {
                    v.as_array().map(|arr| {
                        arr.iter()
                            .filter_map(|s| s.as_str().map(String::from))
                            .collect()
                    })
                },
            },
        })
        .await
        .context(AppError::Settings)?
    } else {
        // No --san: check if DB has a value
        let db_sans = global_raw.get_setting(SettingKey::Sans).and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|s| s.as_str().map(String::from))
                    .collect::<Vec<String>>()
            })
        });
        match db_sans {
            Some(existing) => {
                // Case 2: DB has SANs — use them
                tracing::debug!(count = existing.len(), "using existing SANs from database");
                existing
            }
            None => {
                // Case 3: first start — auto-detect and save
                let detected =
                    uptrakit_web_api::pki_utils::auto_detect_sans().context(AppError::Pki)?;
                let san_strings: Vec<String> = detected
                    .dns_names
                    .into_iter()
                    .chain(detected.ip_addrs.iter().map(|ip| ip.to_string()))
                    .collect();
                tracing::info!(
                    sans = ?san_strings,
                    "first start: auto-detected SANs, saving to database"
                );
                uptrakit_web_api::settings_store::upsert_global_setting(
                    db,
                    SettingKey::Sans,
                    serde_json::json!(san_strings),
                )
                .await
                .context(AppError::Settings)?;
                san_strings
            }
        }
    };
    settings.set_sans(sans.clone()).await;

    let https_addr = reconcile_socket_addr(
        db,
        SettingKey::HttpsAddr,
        global_raw,
        args.https_addr,
        uptrakit_web_api::settings::DEFAULT_HTTPS_ADDR
            .parse()
            .map_err(|e| {
                report!(AppError::Config(format!(
                    "invalid default HTTPS address constant: {e}"
                ),))
            })?,
        force,
    )
    .await?;
    settings.set_https_addr(https_addr).await;

    // NATS URL reconciliation (only when nats feature is enabled)
    #[cfg(feature = "nats")]
    let nats_url = {
        let nats_url_raw = crate::reconcile::reconcile_setting(crate::reconcile::ReconcileParams {
            db,
            key: SettingKey::NatsUrl,
            raw: global_raw,
            cli_value: args.nats_url.clone(),
            default_value: String::new(),
            force,
            convert: crate::reconcile::JsonConvert {
                to_json: |v| {
                    if v.is_empty() {
                        return serde_json::Value::Null;
                    }
                    uptrakit_crypto::encrypt_str(v, "uptrakit:settings:nats_url")
                        .map(|e| serde_json::json!(e))
                        .unwrap_or_else(|_| serde_json::json!(v))
                },
                from_json: |v| {
                    let s = v.as_str().filter(|s| !s.is_empty())?;
                    if uptrakit_crypto::is_encrypted(s) {
                        uptrakit_crypto::decrypt_str(s, "uptrakit:settings:nats_url")
                            .map_err(|e| {
                                tracing::warn!("failed to decrypt nats.url: {e}");
                            })
                            .ok()
                    } else {
                        Some(s.to_string())
                    }
                },
            },
        })
        .await
        .context(AppError::Settings)?;

        let nats_url_opt: Option<String> = if nats_url_raw.is_empty() {
            None
        } else {
            Some(nats_url_raw)
        };
        settings
            .set_nats_url(nats_url_opt.as_deref().map(MaskedUrl::new))
            .await;
        nats_url_opt
    };

    // Zeroconf settings reconciliation (only when zeroconf feature is enabled)
    #[cfg(feature = "zeroconf")]
    {
        let zeroconf_enabled =
            crate::reconcile::reconcile_setting(crate::reconcile::ReconcileParams {
                db,
                key: SettingKey::ZeroconfEnabled,
                raw: global_raw,
                cli_value: if args.zeroconf { Some(true) } else { None },
                default_value: false,
                force,
                convert: crate::reconcile::JsonConvert {
                    to_json: |v| serde_json::json!(v),
                    from_json: |v| v.as_bool(),
                },
            })
            .await
            .context(AppError::Settings)?;

        let zeroconf_url_opt = reconcile_nullable_string(
            db,
            SettingKey::ZeroconfUrl,
            global_raw,
            args.zeroconf_url.clone(),
            force,
        )
        .await?;

        // Fall back to reconciled pki_addr if no explicit zeroconf_pki_addr
        let zeroconf_pki_addr_cli = args
            .zeroconf_pki_addr
            .clone()
            .or_else(|| pki_addr_opt.clone());
        let zeroconf_pki_addr_opt = reconcile_nullable_string(
            db,
            SettingKey::ZeroconfPkiAddr,
            global_raw,
            zeroconf_pki_addr_cli,
            force,
        )
        .await?;

        let zeroconf_snapshot = uptrakit_web_api::settings::ZeroconfSnapshot {
            enabled: zeroconf_enabled,
            url: zeroconf_url_opt,
            pki_addr: zeroconf_pki_addr_opt,
        };
        settings.set_zeroconf(zeroconf_snapshot).await;
    }

    Ok(ReconciledSettings {
        sans,
        pki_addr: pki_addr_opt,
        https_addr,
        #[cfg(feature = "nats")]
        nats_url,
    })
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Check whether a trusted proxy CIDR is overly broad and emit a warning.
///
/// Broad CIDRs in the trusted-proxy list effectively trust a large portion of the
/// internet to set the `X-Forwarded-For` header, which undermines IP-based rate
/// limiting and audit logging.
fn warn_broad_trusted_proxy(cidr: &IpNet) {
    let prefix = cidr.prefix_len();
    if prefix == 0 {
        tracing::warn!(
            cidr = %cidr,
            "trusted proxy CIDR has prefix length /0 — this trusts ALL IP addresses to set \
             forwarded headers, effectively disabling IP-based security controls"
        );
    } else if is_broad_trusted_proxy(cidr) {
        tracing::warn!(
            cidr = %cidr,
            "trusted proxy CIDR is very broad — consider narrowing to specific proxy subnets"
        );
    }
}

/// Returns `true` when the CIDR is broad enough to warrant a security warning.
///
/// Thresholds: IPv4 /8 or less, IPv6 /32 or less, or /0 for either family.
fn is_broad_trusted_proxy(cidr: &IpNet) -> bool {
    let prefix = cidr.prefix_len();
    match cidr {
        IpNet::V4(_) => prefix <= 8,
        IpNet::V6(_) => prefix <= 32,
    }
}

/// Wrapper for `&[T]` that implements Display for logging in reconciliation.
struct DisplayVec<'a, T: fmt::Display>(&'a [T]);

impl<T: fmt::Display> fmt::Display for DisplayVec<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            write!(f, "[]")
        } else {
            let items: Vec<String> = self.0.iter().map(|i| i.to_string()).collect();
            write!(f, "[{}]", items.join(", "))
        }
    }
}

/// Reconcile a `Vec<T>` global setting.  Empty CLI vec is treated as "not provided".
async fn reconcile_setting_vec<T>(
    params: crate::reconcile::ReconcileParams<'_, Vec<T>>,
) -> crate::reconcile::Result<Vec<T>>
where
    T: PartialEq + Clone + fmt::Display + 'static,
{
    let crate::reconcile::ReconcileParams {
        db,
        key,
        raw,
        cli_value,
        default_value,
        force,
        convert,
    } = params;
    let db_key = key.as_str();
    let db_value = raw.get(db_key).and_then(convert.from_json);

    match (db_value, cli_value) {
        (Some(db_val), Some(cli_val)) if db_val != cli_val => {
            if force {
                tracing::info!(key = db_key, cli = %DisplayVec(&cli_val), db = %DisplayVec(&db_val), "force-overriding DB setting with CLI value");
                uptrakit_web_api::settings_store::upsert_global_setting(
                    db,
                    key,
                    (convert.to_json)(&cli_val),
                )
                .await
                .map_err(|e| {
                    tracing::error!(key = db_key, error = ?e, "failed to upsert setting");
                    rootcause::report!(crate::reconcile::ReconcileError)
                })?;
                Ok(cli_val)
            } else {
                tracing::warn!(
                    key = db_key,
                    cli = %DisplayVec(&cli_val),
                    db = %DisplayVec(&db_val),
                    "CLI value differs from DB; using DB value (pass --force-settings-override to overwrite)"
                );
                Ok(db_val)
            }
        }
        (Some(db_val), _) => {
            tracing::debug!(key = db_key, value = %DisplayVec(&db_val), "using DB value");
            Ok(db_val)
        }
        (None, Some(cli_val)) => {
            tracing::info!(key = db_key, value = %DisplayVec(&cli_val), "seeding DB setting from CLI");
            uptrakit_web_api::settings_store::upsert_global_setting(
                db,
                key,
                (convert.to_json)(&cli_val),
            )
            .await
            .map_err(|e| {
                tracing::error!(key = db_key, error = ?e, "failed to upsert setting");
                rootcause::report!(crate::reconcile::ReconcileError)
            })?;
            Ok(cli_val)
        }
        (None, None) => {
            tracing::info!(key = db_key, value = %DisplayVec(&default_value), "seeding DB setting from default");
            uptrakit_web_api::settings_store::upsert_global_setting(
                db,
                key,
                (convert.to_json)(&default_value),
            )
            .await
            .map_err(|e| {
                tracing::error!(key = db_key, error = ?e, "failed to upsert setting");
                rootcause::report!(crate::reconcile::ReconcileError)
            })?;
            Ok(default_value)
        }
    }
}

/// Reconcile a `SocketAddr` global setting.
async fn reconcile_socket_addr(
    db: &sea_orm::DatabaseConnection,
    key: SettingKey,
    raw: &uptrakit_web_api::settings_store::RawSettings,
    cli_value: Option<SocketAddr>,
    default_value: SocketAddr,
    force: bool,
) -> crate::Result<SocketAddr> {
    crate::reconcile::reconcile_setting(crate::reconcile::ReconcileParams {
        db,
        key,
        raw,
        cli_value,
        default_value,
        force,
        convert: crate::reconcile::JsonConvert {
            to_json: |v| serde_json::json!(v.to_string()),
            from_json: |v| v.as_str().and_then(|s| s.parse().ok()),
        },
    })
    .await
    .context(AppError::Settings)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[test]
    fn broad_proxy_ipv4_slash_zero() {
        let cidr: ipnet::IpNet = "0.0.0.0/0".parse().unwrap();
        assert!(super::is_broad_trusted_proxy(&cidr));
    }

    #[test]
    fn broad_proxy_ipv6_slash_zero() {
        let cidr: ipnet::IpNet = "::/0".parse().unwrap();
        assert!(super::is_broad_trusted_proxy(&cidr));
    }

    #[test]
    fn broad_proxy_ipv4_slash_eight() {
        let cidr: ipnet::IpNet = "10.0.0.0/8".parse().unwrap();
        assert!(super::is_broad_trusted_proxy(&cidr));
    }

    #[test]
    fn narrow_proxy_ipv4_slash_sixteen() {
        let cidr: ipnet::IpNet = "192.168.0.0/16".parse().unwrap();
        assert!(!super::is_broad_trusted_proxy(&cidr));
    }

    #[test]
    fn narrow_proxy_ipv4_slash_thirtytwo() {
        let cidr: ipnet::IpNet = "127.0.0.1/32".parse().unwrap();
        assert!(!super::is_broad_trusted_proxy(&cidr));
    }

    #[test]
    fn broad_proxy_ipv6_slash_thirtytwo() {
        let cidr: ipnet::IpNet = "2001:db8::/32".parse().unwrap();
        assert!(super::is_broad_trusted_proxy(&cidr));
    }

    #[test]
    fn narrow_proxy_ipv6_slash_fortyeight() {
        let cidr: ipnet::IpNet = "2001:db8:abcd::/48".parse().unwrap();
        assert!(!super::is_broad_trusted_proxy(&cidr));
    }
}
