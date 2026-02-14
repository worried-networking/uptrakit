mod cli;
mod client;
mod error;
mod host_info;
mod update;
mod version_check;

use clap::Parser;
use rootcause::prelude::*;
use tracing_subscriber::EnvFilter;
use uptrakit_build_info::BuildInfo;
use uptrakit_service_sdk::{
    AuthenticatedContext, LoopOutcome, ServiceConfig, ServiceEnrollmentInfo, ServiceHandler,
};

use cli::Args;

struct AgentHandler;

impl ServiceHandler for AgentHandler {
    fn config(&self) -> ServiceConfig {
        ServiceConfig {
            dir_name: "agent",
            service_label: "uptrakit-agent service",
        }
    }

    fn enrollment_info(&self) -> ServiceEnrollmentInfo {
        ServiceEnrollmentInfo {
            service_type: uptrakit_internal_wire::ServiceType::Agent,
        }
    }

    fn run_authenticated_loop<'a>(
        &'a mut self,
        ctx: AuthenticatedContext<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = uptrakit_service_sdk::Result<LoopOutcome>> + Send + 'a>,
    > {
        Box::pin(async move {
            let cert_not_after_ts = ctx.identity.cert_not_after_ms();

            match client::run_authenticated_loop(client::AuthenticatedLoopParams {
                host: ctx.host,
                port: ctx.port,
                base_url: ctx.base_url,
                pki_addr: ctx.pki_addr,
                ca_pem: ctx.ca_pem,
                tls_connector: ctx.tls_connector,
                cert_not_after_ts,
                identity: ctx.identity,
            })
            .await
            {
                Ok(outcome) => Ok(outcome),
                Err(e) => {
                    // Convert agent error to SDK error, preserving cert-expired
                    // and receive-closed semantics for the lifecycle.
                    let ctx = e.current_context();
                    if ctx.is_cert_expired() {
                        Err(report!(
                            uptrakit_service_sdk::EnrollmentError::Rustls(
                                rustls::Error::AlertReceived(
                                    rustls::AlertDescription::CertificateExpired,
                                )
                            )
                        ))
                    } else if ctx.is_receive_closed() {
                        Err(report!(
                            uptrakit_service_sdk::EnrollmentError::ReceiveClosed
                        ))
                    } else {
                        Err(report!(
                            uptrakit_service_sdk::EnrollmentError::Enrollment(e.to_string())
                        ))
                    }
                }
            }
        })
    }
}

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

    let mut handler = AgentHandler;
    if let Err(e) = uptrakit_service_sdk::run_service_lifecycle(&args.common, &mut handler).await {
        if e.current_context().is_receive_closed() {
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
