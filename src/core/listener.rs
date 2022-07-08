use std::{net::SocketAddr, sync::Arc};

use tokio::{io::AsyncWriteExt, net::TcpListener, sync::watch};
use tracing::{debug, error};

use crate::core::{
    client::Session,
    connection::{handle_conn, handle_conn_tls},
};

use super::Core;

static SERVER_GREETING: &[u8] = concat!(
    "* OK Stalwart IMAP4rev2 v",
    env!("CARGO_PKG_VERSION"),
    " at your service.\r\n"
)
.as_bytes();

pub async fn spawn_listener(
    bind_addr: SocketAddr,
    core: Arc<Core>,
    is_tls: bool,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    // Start listening for IMAP connections.
    let listener = TcpListener::bind(bind_addr).await.unwrap_or_else(|e| {
        panic!("Failed to bind to {}: {}", bind_addr, e);
    });

    tokio::spawn(async move {
        loop {
            tokio::select! {
                stream = listener.accept() => {
                    match stream {
                        Ok((mut stream, _)) => {
                            let shutdown_rx = shutdown_rx.clone();
                            let core = core.clone();
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
                                    if let Err(err) = stream.write_all(SERVER_GREETING).await {
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
                                    if let Err(err) = stream.write_all(SERVER_GREETING).await {
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
