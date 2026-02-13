mod cli;
mod client;
mod error;
mod host_info;
mod update;
mod version_check;

use std::time::Duration;

use clap::Parser;
use rootcause::prelude::*;
use tracing_subscriber::EnvFilter;
use uptrakit_build_info::BuildInfo;
use uptrakit_service_sdk::ServiceIdentityState;

use cli::Args;
use client::LoopOutcome;
use error::Error;

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if args.common.version {
        print_build_info();
        return;
    }

    let filter = match "uptrakit_agent=info".parse() {
        Ok(directive) => EnvFilter::from_default_env().add_directive(directive),
        Err(_) => EnvFilter::from_default_env(),
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Install the default crypto provider for rustls
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    if let Err(mut e) = run(&args).await {
        if e.current_context_mut().is_receive_closed() {
            tracing::info!("disconnected by controller");
        } else {
            tracing::error!(error = %e, "agent failed");
            std::process::exit(1);
        }
    }
}

fn print_build_info() {
    let build_info = BuildInfo::current(
        "uptrakit-agent",
        env!("CARGO_PKG_VERSION"),
        option_env!("UPTRAKIT_BUILD_ENABLED_FEATURES"),
    );
    let output = build_info.render_human();
    print!("{output}");
}

async fn run(args: &Args) -> error::Result<()> {
    tracing::info!("starting uptrakit-agent service");

    // Parse URL early
    let (host, port) = args.common.parsed_url().map_err(|s| {
        report!(Error::Enrollment(
            uptrakit_service_sdk::EnrollmentError::Enrollment(s)
        ))
    })?;
    let base_url = args.common.base_url();
    let pki_addr = args.common.pki_addr();

    // Resolve application directories
    let app_dirs = args.common.resolve_dirs("agent").map_err(|e| {
        report!(Error::Enrollment(
            uptrakit_service_sdk::EnrollmentError::Enrollment(e.to_string())
        ))
    })?;
    app_dirs.ensure_dirs().map_err(|e| {
        report!(Error::Enrollment(
            uptrakit_service_sdk::EnrollmentError::Enrollment(format!(
                "failed to create directories: {e}"
            ))
        ))
    })?;
    tracing::info!("config directory: {}", app_dirs.config_dir().display());
    tracing::info!("state directory: {}", app_dirs.state_dir().display());

    // Create and load identity state
    let mut identity = ServiceIdentityState::new(app_dirs.config_dir(), app_dirs.state_dir());
    identity.load().await.context_to::<Error>()?;

    // --force-enroll: clear existing enrollment state (preserves CA cert)
    if args.common.force_enroll {
        tracing::info!("--force-enroll: clearing existing enrollment state");
        identity
            .clear_enrollment_state()
            .await
            .context_to::<Error>()?;
    }

    // CA bootstrap: cached → --ca-cert file → --pki-addr → --tofu TOFU → system trust
    let ca_pem = uptrakit_service_sdk::ca::bootstrap_ca(
        &mut identity,
        base_url,
        args.common.tofu,
        args.common.tofu_fingerprint.as_deref(),
        args.common.ca_cert.as_deref(),
        pki_addr,
    )
    .await
    .context_to::<Error>()?;

    // Check for existing certificate
    if identity.is_certified() {
        let cert_not_after_ts = identity.cert_not_after_ms();
        let cert_expired =
            cert_not_after_ts.is_some_and(|ts| uptrakit_internal_wire::now_millis() >= ts);

        if cert_expired {
            tracing::warn!("certificate expired, falling back to fresh enrollment");
            identity
                .clear_enrollment_state()
                .await
                .context_to::<Error>()?;
            // Fall through to enrollment below
        } else {
            tracing::info!("loaded existing certificate from disk");
            match run_authenticated_with_reconnect(
                &host,
                port,
                base_url,
                pki_addr,
                ca_pem.as_deref(),
                &identity,
            )
            .await
            {
                Ok(()) => return Ok(()),
                Err(mut e) => {
                    if e.current_context_mut().is_cert_expired() {
                        tracing::warn!("certificate expired, falling back to enrollment");
                        identity
                            .clear_enrollment_state()
                            .await
                            .context_to::<Error>()?;
                        // Fall through to enrollment below
                    } else {
                        return Err(e);
                    }
                }
            }
        }
    }

    // Enrollment (existing service.json OR fresh enrollment)
    // Retry on disconnect — e.g. a merge transfers our identity while we wait.
    let tls_connector = match ca_pem.as_deref() {
        Some(pem) => uptrakit_service_sdk::tls::build_tls_connector(pem).context_to::<Error>()?,
        None => {
            uptrakit_service_sdk::tls::build_system_trust_tls_connector().context_to::<Error>()?
        }
    };

    let mut enrollment_backoff =
        uptrakit_service_sdk::Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
    loop {
        match do_enrollment(args, &host, port, &mut identity, &tls_connector).await {
            Ok(()) => break,
            Err(mut e) => {
                if e.current_context_mut().is_receive_closed() {
                    let delay = enrollment_backoff.next_delay();
                    tracing::info!("disconnected during enrollment, reconnecting in {delay:?}");
                    tokio::time::sleep(delay).await;
                    // Reload identity in case enrollment partially completed
                    identity.load().await.context_to::<Error>()?;
                    continue;
                }
                return Err(e);
            }
        }
    }

    // Enter mTLS loop with reconnect
    run_authenticated_with_reconnect(
        &host,
        port,
        base_url,
        pki_addr,
        ca_pem.as_deref(),
        &identity,
    )
    .await
}

/// Run enrollment using the shared enrollment crate.
async fn do_enrollment(
    args: &Args,
    host: &str,
    port: u16,
    identity: &mut ServiceIdentityState,
    tls_connector: &tokio_rustls::TlsConnector,
) -> error::Result<()> {
    if identity.is_enrolled_only() {
        // Resume: reconnect with Bearer header (existing service.json)
        tracing::info!("reconnecting with enrollment secret");
        uptrakit_service_sdk::ws::resume_enrollment(identity, host, port, tls_connector)
            .await
            .context_to::<Error>()?;
    } else {
        // Fresh enrollment
        let hostname = args.common.hostname();
        let friendly_name = args.common.friendly_name_or_hostname();
        let host_info = host_info::collect_host_info();
        tracing::info!(machine_id = %host_info.machine_id, "collected host info");

        tracing::info!("enrolling via WebSocket");
        uptrakit_service_sdk::ws::run_enrollment(uptrakit_service_sdk::ws::EnrollmentParams {
            identity,
            host,
            port,
            tls_connector,
            hostname: &hostname,
            friendly_name: &friendly_name,
            enrollment_token: args.common.enrollment_token.as_deref(),
            service_type: uptrakit_internal_wire::ServiceType::Agent,
            host_info: Some(host_info),
        })
        .await
        .context_to::<Error>()?;
    }

    tracing::info!("enrollment complete, certificate saved to disk");
    Ok(())
}

/// Enter the mTLS authenticated loop with automatic reconnection on cert rotation.
async fn run_authenticated_with_reconnect(
    host: &str,
    port: u16,
    base_url: &str,
    pki_addr: Option<&str>,
    ca_pem: Option<&[u8]>,
    identity: &ServiceIdentityState,
) -> error::Result<()> {
    let mut reconnect_backoff =
        uptrakit_service_sdk::Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
    loop {
        let cert_pem = identity.cert_pem().ok_or_else(|| {
            report!(Error::Enrollment(
                uptrakit_service_sdk::EnrollmentError::NotCertified
            ))
        })?;
        let key_pem = identity.key_pem().ok_or_else(|| {
            report!(Error::Enrollment(
                uptrakit_service_sdk::EnrollmentError::NotCertified
            ))
        })?;
        let cert_not_after_ts = identity.cert_not_after_ms();

        let mtls_connector = match ca_pem {
            Some(pem) => uptrakit_service_sdk::tls::build_tls_connector_with_client_cert(
                pem, cert_pem, &key_pem,
            )
            .context_to::<Error>()?,
            None => uptrakit_service_sdk::tls::build_system_trust_tls_connector_with_client_cert(
                cert_pem, &key_pem,
            )
            .context_to::<Error>()?,
        };

        match client::run_authenticated_loop(client::AuthenticatedLoopParams {
            host,
            port,
            base_url,
            pki_addr,
            ca_pem,
            tls_connector: mtls_connector,
            cert_not_after_ts,
            identity,
        })
        .await?
        {
            LoopOutcome::Shutdown => return Ok(()),
            LoopOutcome::Reconnect => {
                // Certificate rotation is expected; reset backoff and reconnect quickly.
                reconnect_backoff.reset();
                tracing::info!("reconnecting with new certificate");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            LoopOutcome::Disconnected => {
                let delay = reconnect_backoff.next_delay();
                tracing::warn!("disconnected by controller, reconnecting in {delay:?}");
                tokio::time::sleep(delay).await;
                continue;
            }
            LoopOutcome::Restart => {
                tracing::info!("restart requested, exiting for external restart");
                return Ok(());
            }
        }
    }
}
