/*
 * Copyright (c) 2020-2022, Stalwart Labs Ltd.
 *
 * This file is part of the Stalwart IMAP Server.
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of
 * the License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 * in the LICENSE file at the top-level directory of this distribution.
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/>.
 *
 * You can be released from the requirements of the AGPLv3 license by
 * purchasing a commercial license. Please contact licensing@stalw.art
 * for more details.
*/

pub mod commands;
pub mod core;
pub mod managesieve;
pub mod parser;
pub mod protocol;
#[cfg(test)]
pub mod tests;

use crate::{
    core::{
        config::{build_core, failed_to, UnwrapFailure},
        env_settings::EnvSettings,
        housekeeper::spawn_housekeeper,
    },
    managesieve::listener::spawn_managesieve_listener,
};
use std::{net::SocketAddr, sync::Arc, time::Duration};

use tokio::sync::watch;
use tracing::{debug, info, Level};

use crate::core::listener::spawn_listener;

const IMAP4_PORT: u16 = 143;
const IMAP4_PORT_TLS: u16 = 993;
const MANAGESIEVE_PORT: u16 = 4190;

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
                "Starting Stalwart IMAP server v{} at {}{}...",
                env!("CARGO_PKG_VERSION"),
                socket_addr,
                if is_tls { " (TLS)" } else { "" }
            );
            spawn_listener(socket_addr, core.clone(), is_tls, shutdown_rx.clone()).await;
        }
    }

    // Start ManageSieve listener
    if let Some(bind_port) = settings.get("bind-port-managesieve") {
        let socket_addr =
            SocketAddr::from((bind_addr, bind_port.parse().unwrap_or(MANAGESIEVE_PORT)));
        info!(
            "Starting Stalwart ManageSieve server v{} at {}...",
            env!("CARGO_PKG_VERSION"),
            socket_addr,
        );
        spawn_managesieve_listener(socket_addr, core.clone(), shutdown_rx.clone()).await;
    }

    // Start houskeeper
    spawn_housekeeper(core, &settings, shutdown_rx);

    // Wait for shutdown signal
    #[cfg(not(target_env = "msvc"))]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut h_term = signal(SignalKind::terminate()).failed_to("start signal handler");
        let mut h_int = signal(SignalKind::interrupt()).failed_to("start signal handler");

        tokio::select! {
            _ = h_term.recv() => debug!("Received SIGTERM."),
            _ = h_int.recv() => debug!("Received SIGINT."),
        };
    }

    #[cfg(target_env = "msvc")]
    {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {}
            Err(err) => {
                eprintln!("Unable to listen for shutdown signal: {}", err);
            }
        }
    }

    // Shutdown the system;
    info!(
        "Shutting down Stalwart IMAP server v{}...",
        env!("CARGO_PKG_VERSION")
    );
    shutdown_tx.send(true).unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;

    Ok(())
}
