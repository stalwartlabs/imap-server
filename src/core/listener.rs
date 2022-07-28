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

use super::{Core, ResponseCode, StatusResponse};

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
        panic!("Failed to bind to {}: {}", bind_addr, e);
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
                                    if let Err(err) = stream.write_all(&greeting).await {
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
                                    if let Err(err) = stream.write_all(&greeting_tls).await {
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
