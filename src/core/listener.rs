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

use std::{net::SocketAddr, sync::Arc};

use tokio::{io::AsyncWriteExt, net::TcpListener, sync::watch};
use tracing::{debug, error};

use crate::{
    core::{
        client::Session,
        connection::{handle_conn, handle_conn_tls},
    },
    protocol::capability::Capability,
};

use super::{config::failed_to, Core, ResponseCode, StatusResponse};

static SERVER_GREETING: &str = concat!(
    "Stalwart IMAP4rev2 v",
    env!("CARGO_PKG_VERSION"),
    " at your service."
);

pub async fn spawn_listener(
    bind_addr: SocketAddr,
    core: Arc<Core>,
    is_tls: bool,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    // Start listening for IMAP connections.
    let listener = TcpListener::bind(bind_addr).await.unwrap_or_else(|e| {
        failed_to(&format!("bind to {}: {}", bind_addr, e));
    });

    tokio::spawn(async move {
        let greeting = Arc::new(
            StatusResponse::ok(SERVER_GREETING)
                .with_code(ResponseCode::Capability {
                    capabilities: Capability::all_capabilities(false, false),
                })
                .into_bytes(),
        );
        let greeting_tls = Arc::new(
            StatusResponse::ok(SERVER_GREETING)
                .with_code(ResponseCode::Capability {
                    capabilities: Capability::all_capabilities(false, true),
                })
                .into_bytes(),
        );

        loop {
            tokio::select! {
                stream = listener.accept() => {
                    match stream {
                        Ok((mut stream, _)) => {
                            let shutdown_rx = shutdown_rx.clone();
                            let core = core.clone();
                            let greeting = greeting.clone();
                            let greeting_tls = greeting_tls.clone();

                            tokio::spawn(async move {
                                let peer_addr = stream.peer_addr().unwrap();

                                if is_tls {
                                    let mut stream = match core.tls_acceptor.accept(stream).await {
                                        Ok(stream) => stream,
                                        Err(e) => {
                                            debug!("Failed to accept TLS connection: {}", e);
                                            return;
                                        }
                                    };

                                    // Send greeting
                                    if let Err(err) = stream.write_all(&greeting_tls).await {
                                        debug!("Failed to send greeting to {}: {}", peer_addr, err);
                                        return;
                                    }

                                    handle_conn_tls(
                                        stream,
                                        Session::new(core, peer_addr, true),
                                        shutdown_rx
                                    ).await;
                                } else {
                                    // Send greeting
                                    if let Err(err) = stream.write_all(&greeting).await {
                                        debug!("Failed to send greeting to {}: {}", peer_addr, err);
                                        return;
                                    }

                                    handle_conn(
                                        stream,
                                        Session::new(core, peer_addr, false),
                                        shutdown_rx
                                    ).await;
                                }
                            });
                        }
                        Err(err) => {
                            error!("Failed to accept TCP connection: {}", err);
                        }
                    }
                },
                _ = shutdown_rx.changed() => {
                    debug!("IMAP listener shutting down.");
                    break;
                }
            };
        }
    });
}
