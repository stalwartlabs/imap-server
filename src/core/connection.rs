use tokio::{io::AsyncReadExt, net::TcpStream, sync::watch};
use tokio_rustls::server::TlsStream;
use tracing::debug;

use super::client::Session;

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
            result = stream_rx.read(&mut buf) => {
                match result {
                    Ok(bytes_read) => {
                        if bytes_read > 0 {
                            match session.ingest(&buf[..bytes_read]).await {
                                Ok(Some(stream_tx)) => {
                                    debug!("TLS upgrade requested.");
                                    handle_conn_tls(
                                        match session.config.tls_acceptor.accept(stream_rx.unsplit(stream_tx)).await {
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
                    Err(err) => {
                        debug!("IMAP connection closed by peer {}: {}.", session.peer_addr, err);
                        break;
                    },
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
            result = stream_rx.read(&mut buf) => {
                match result {
                    Ok(bytes_read) => {
                        if bytes_read > 0 {
                            if session.ingest(&buf[..bytes_read]).await.is_err() {
                                debug!("Disconnecting client.");
                                return;
                            }
                        } else {
                            debug!("IMAP connection closed by {}", session.peer_addr);
                            break;
                        }
                    },
                    Err(err) => {
                        debug!("IMAP connection closed by peer {}: {}.", session.peer_addr, err);
                        break;
                    },
                }
            },
            _ = shutdown_rx.changed() => {
                debug!("IMAP connection with peer {} shutting down.", session.peer_addr);
                return;
            }
        };
    }
}
