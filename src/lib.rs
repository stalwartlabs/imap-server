pub mod commands;
pub mod core;
pub mod parser;
pub mod protocol;
#[cfg(test)]
pub mod tests;

use crate::core::{
    config::{build_core, failed_to, UnwrapFailure},
    env_settings::EnvSettings,
    housekeeper::spawn_housekeeper,
};
use std::{net::SocketAddr, sync::Arc, time::Duration};

use futures::stream::StreamExt;

use signal_hook::consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
use signal_hook_tokio::Signals;
use tokio::sync::watch;
use tracing::{info, Level};

use crate::core::listener::spawn_listener;

const IMAP4_PORT: u16 = 143;
const IMAP4_PORT_TLS: u16 = 993;

pub async fn start_imap_server(settings: EnvSettings) -> std::io::Result<()> {
    // Enable logging
    tracing::subscriber::set_global_default(
        tracing_subscriber::FmtSubscriber::builder()
            .with_max_level(settings.parse("log-level").unwrap_or(Level::ERROR))
            .finish(),
    )
    .failed_to("set default subscriber.");

    // Read configuration parameters
    if !settings.contains_key("bind-port") && !settings.contains_key("bind-port-tls") {
        failed_to("start IMAP listener. Please specify 'bind-port' and/or 'bind-port-tls'.");
    }
    let core = Arc::new(build_core(&settings));

    // Start IMAP listeners
    let bind_addr = settings.parse_ipaddr("bind-addr", "0.0.0.0");
    let (shutdown_tx, shutdown_rx) = watch::channel(true);
    for (pos, bind_port) in ["bind-port", "bind-port-tls"].into_iter().enumerate() {
        if let Some(bind_port) = settings.get(bind_port) {
            let is_tls = pos > 0;
            let socket_addr = SocketAddr::from((
                bind_addr,
                bind_port
                    .parse()
                    .unwrap_or(if is_tls { IMAP4_PORT_TLS } else { IMAP4_PORT }),
            ));
            info!(
                "Starting Stalwart IMAP4rev2 server at {}{}...",
                socket_addr,
                if is_tls { " (TLS)" } else { "" }
            );
            spawn_listener(socket_addr, core.clone(), is_tls, shutdown_rx.clone()).await;
        }
    }

    // Start houskeeper
    spawn_housekeeper(core, &settings, shutdown_rx);

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
