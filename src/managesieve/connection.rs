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

use std::time::Duration;

use tokio::{io::AsyncReadExt, net::TcpStream, sync::watch};
use tokio_rustls::server::TlsStream;
use tracing::debug;

use crate::managesieve::client::State;

use super::client::Session;

const NON_AUTHENTICATED_TIMEOUT: Duration = Duration::from_secs(60);
const AUTHENTICATED_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub async fn handle_conn(
    stream: TcpStream,
    mut session: Session,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut buf = vec![0; 4096];
    let (mut stream_rx, stream_tx) = tokio::io::split(stream);

    if !session.set_stream(stream_tx).await {
        return;
    }

    loop {
        tokio::select! {
            result = tokio::time::timeout(
                if !matches!(session.state, State::NotAuthenticated {..}) {
                    AUTHENTICATED_TIMEOUT
                } else {
                    NON_AUTHENTICATED_TIMEOUT
                },
                stream_rx.read(&mut buf)) => {
                match result {
                    Ok(Ok(bytes_read)) => {
                        if bytes_read > 0 {
                            match session.ingest(&buf[..bytes_read]).await {
                                Ok(Some(stream_tx)) => {
                                    debug!("TLS upgrade requested.");
                                    handle_conn_tls(
                                        match session.core.tls_acceptor.accept(stream_rx.unsplit(stream_tx)).await {
                                            Ok(stream) => stream,
                                            Err(e) => {
                                                debug!("Failed to accept TLS connection: {}", e);
                                                return;
                                            }
                                        },
                                        session,
                                        shutdown_rx,
                                    )
                                    .await;
                                    return;
                                }
                                Ok(None) => (),
                                Err(_) => {
                                    debug!("Disconnecting client.");
                                    return;
                                }
                            }
                        } else {
                            debug!("ManageSieve connection closed by {}", session.peer_addr);
                            break;
                        }
                    },
                    Ok(Err(err)) => {
                        debug!("ManageSieve connection closed by {}: {}.", session.peer_addr, err);
                        break;
                    },
                    Err(_) => {
                        session.write_bytes(b"* BYE Connection timed out.\r\n".to_vec()).await.ok();
                        debug!("ManageSieve connection timed out with {}.", session.peer_addr);
                        break;
                    }
                }
            },
            _ = shutdown_rx.changed() => {
                session.write_bytes(b"* BYE Server shutting down.\r\n".to_vec()).await.ok();
                debug!("ManageSieve connection with peer {} shutting down.", session.peer_addr);
                return;
            }
        };
    }
}

pub async fn handle_conn_tls(
    stream: TlsStream<TcpStream>,
    mut session: Session,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut buf = vec![0; 4096];
    let (mut stream_rx, stream_tx) = tokio::io::split(stream);

    if !session.set_stream_tls(stream_tx).await {
        return;
    }

    loop {
        tokio::select! {
            result = tokio::time::timeout(
                if !matches!(session.state, State::NotAuthenticated {..}) {
                    AUTHENTICATED_TIMEOUT
                } else {
                    NON_AUTHENTICATED_TIMEOUT
                },
                stream_rx.read(&mut buf)) => {
                match result {
                    Ok(Ok(bytes_read)) => {
                        if bytes_read > 0 {
                            if session.ingest(&buf[..bytes_read]).await.is_err() {
                                debug!("Disconnecting client.");
                                return;
                            }
                        } else {
                            debug!("ManageSieve connection closed by {}", session.peer_addr);
                            break;
                        }
                    },
                    Ok(Err(err)) => {
                        debug!("ManageSieve connection closed by peer {}: {}.", session.peer_addr, err);
                        break;
                    },
                    Err(_) => {
                        session.write_bytes(b"* BYE Connection timed out.\r\n".to_vec()).await.ok();
                        debug!("ManageSieve connection timed out with {}.", session.peer_addr);
                        break;
                    }
                }
            },
            _ = shutdown_rx.changed() => {
                session.write_bytes(b"* BYE Server shutting down.\r\n".to_vec()).await.ok();
                debug!("ManageSieve connection with peer {} shutting down.", session.peer_addr);
                return;
            }
        };
    }
}
