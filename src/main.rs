use crate::core::config::{load_config, UnwrapFailure};
use crate::core::listener::spawn_listener;
use std::{net::SocketAddr, sync::Arc, time::Duration};

use futures::stream::StreamExt;

use signal_hook::consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
use signal_hook_tokio::Signals;
use tokio::sync::watch;
use tracing::{info, Level};

use crate::core::env_settings::EnvSettings;

pub mod commands;
pub mod core;
pub mod parser;
pub mod protocol;
#[cfg(test)]
pub mod tests;

const IMAP4_PORT: u16 = 143;
const IMAP4_PORT_TLS: u16 = 993;

async fn start_imap_server(settings: EnvSettings) -> std::io::Result<()> {
    // Enable logging
    tracing::subscriber::set_global_default(
        tracing_subscriber::FmtSubscriber::builder()
            .with_max_level(settings.parse("log-level").unwrap_or(Level::ERROR))
            .finish(),
    )
    .failed_to("set default subscriber.");

    // Read configuration parameters
    let bind_addr = SocketAddr::from((
        settings.parse_ipaddr("bind-addr", "127.0.0.1"),
        settings.parse("bind-port").unwrap_or(IMAP4_PORT),
    ));
    let bind_addr_tls = SocketAddr::from((
        settings.parse_ipaddr("bind-addr", "127.0.0.1"),
        settings.parse("bind-port-tls").unwrap_or(IMAP4_PORT_TLS),
    ));
    let (shutdown_tx, shutdown_rx) = watch::channel(true);
    let config = Arc::new(load_config(&settings));

    // Start IMAP server
    info!(
        "Starting Stalwart IMAP4rev2 server at {} + {} (TLS)...",
        bind_addr, bind_addr_tls
    );
    spawn_listener(bind_addr, config.clone(), false, shutdown_rx.clone()).await;
    spawn_listener(bind_addr_tls, config, true, shutdown_rx).await;

    // Wait for shutdown signal
    let mut signals = Signals::new(&[SIGHUP, SIGTERM, SIGINT, SIGQUIT])?;

    while let Some(signal) = signals.next().await {
        match signal {
            SIGHUP => {
                // Reload configuration
            }
            SIGTERM | SIGINT | SIGQUIT => {
                // Shutdown the system;
                info!("Shutting down Stalwart IMAP4rev2 server...");
                shutdown_tx.send(true).unwrap();
                tokio::time::sleep(Duration::from_secs(1)).await;
                break;
            }
            _ => unreachable!(),
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // Start server
    start_imap_server(EnvSettings::new()).await
}
