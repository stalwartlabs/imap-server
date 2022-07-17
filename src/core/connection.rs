use std::time::Duration;

use tokio::{io::AsyncReadExt, net::TcpStream, sync::watch};
use tokio_rustls::server::TlsStream;
use tracing::debug;

use crate::core::client::State;

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
                            debug!("IMAP connection closed by {}", session.peer_addr);
                            break;
                        }
                    },
                    Ok(Err(err)) => {
                        debug!("IMAP connection closed by {}: {}.", session.peer_addr, err);
                        break;
                    },
                    Err(_) => {
                        session.write_bytes(b"* BYE Connection timed out.\r\n".to_vec()).await.ok();
                        debug!("IMAP connection timed out with {}.", session.peer_addr);
                        break;
                    }
                }
            },
            _ = shutdown_rx.changed() => {
                debug!("IMAP connection with peer {} shutting down.", session.peer_addr);
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
                            match &session.idle_tx {
                                None => {
                                    if session.ingest(&buf[..bytes_read]).await.is_err() {
                                        debug!("Disconnecting client.");
                                        return;
                                    }
                                },
                                Some(idle_tx) => {
                                    if bytes_read >= 4 && &buf[..4] == b"DONE" {
                                        debug!("Stopping IDLE.");
                                        idle_tx.send(false).ok();
                                        session.idle_tx = None;
                                    }
                                },
                            }
                        } else {
                            debug!("IMAP connection closed by {}", session.peer_addr);
                            break;
                        }
                    },
                    Ok(Err(err)) => {
                        debug!("IMAP connection closed by peer {}: {}.", session.peer_addr, err);
                        break;
                    },
                    Err(_) => {
                        session.write_bytes(b"* BYE Connection timed out.\r\n".to_vec()).await.ok();
                        debug!("IMAP connection timed out with {}.", session.peer_addr);
                        break;
                    }
                }
            },
            _ = shutdown_rx.changed() => {
                debug!("IMAP connection with peer {} shutting down.", session.peer_addr);
                return;
            }
        };
    }
}
