use std::{net::SocketAddr, sync::Arc};

use jmap_client::client::Client;
use tokio::{io::WriteHalf, net::TcpStream, sync::mpsc};
use tokio_rustls::server::TlsStream;
use tracing::debug;

use crate::protocol::ProtocolVersion;

use super::{
    config::Config,
    receiver::{self, Receiver, Request},
    writer, Command, StatusResponse,
};

pub struct Session {
    pub config: Arc<Config>,
    pub receiver: Receiver,
    pub version: ProtocolVersion,
    pub state: State,
    pub peer_addr: SocketAddr,
    pub is_tls: bool,
    pub writer: mpsc::Sender<writer::Event>,
}

pub struct SessionData {
    pub client: Client,
    pub config: Arc<Config>,
    pub writer: mpsc::Sender<writer::Event>,
}

pub enum State {
    NotAuthenticated { auth_failures: u8 },
    Authenticated { data: Arc<SessionData> },
    Selected { data: Arc<SessionData> },
}

impl Session {
    pub fn new(config: Arc<Config>, peer_addr: SocketAddr, is_tls: bool) -> Self {
        Session {
            config,
            receiver: Receiver::new(),
            version: ProtocolVersion::Rev1,
            state: State::NotAuthenticated { auth_failures: 0 },
            peer_addr,
            is_tls,
            writer: writer::spawn_writer(),
        }
    }

    pub async fn set_stream(&mut self, stream_tx: WriteHalf<TcpStream>) -> bool {
        if let Err(err) = self.writer.send(writer::Event::Stream(stream_tx)).await {
            debug!("Failed to send stream: {}", err);
            false
        } else {
            true
        }
    }

    pub async fn set_stream_tls(&mut self, stream_tx: WriteHalf<TlsStream<TcpStream>>) -> bool {
        self.is_tls = true;
        if let Err(err) = self.writer.send(writer::Event::StreamTls(stream_tx)).await {
            debug!("Failed to send stream: {}", err);
            false
        } else {
            true
        }
    }

    pub async fn ingest(&mut self, bytes: &[u8]) -> Result<Option<WriteHalf<TcpStream>>, ()> {
        let mut bytes = bytes.iter();
        let mut requests = Vec::with_capacity(2);
        let mut needs_literal = None;

        loop {
            match self.receiver.parse(&mut bytes) {
                Ok(request) => match request.is_allowed(&self.state, self.is_tls) {
                    Ok(request) => {
                        requests.push(request);
                    }
                    Err(response) => {
                        self.write_bytes(response.into_bytes()).await?;
                    }
                },
                Err(receiver::Error::NeedsMoreData) => {
                    break;
                }
                Err(receiver::Error::NeedsLiteral { size }) => {
                    needs_literal = size.into();
                    break;
                }
                Err(receiver::Error::Error { response }) => {
                    self.write_bytes(response.into_bytes()).await?;
                    break;
                }
            }
        }

        for request in requests {
            match request.command {
                Command::Capability => {
                    self.handle_capability(request).await?;
                }
                Command::Noop => {
                    self.handle_noop(request).await?;
                }
                Command::Logout => {
                    self.handle_logout(request).await?;
                }
                Command::StartTls => {
                    return self.handle_starttls(request).await;
                }
                Command::Authenticate => {
                    self.handle_authenticate(request).await?;
                }
                Command::Login => {
                    self.handle_login(request).await?;
                }
                Command::Enable => todo!(),
                Command::Select => todo!(),
                Command::Examine => todo!(),
                Command::Create => todo!(),
                Command::Delete => todo!(),
                Command::Rename => todo!(),
                Command::Subscribe => todo!(),
                Command::Unsubscribe => todo!(),
                Command::List => todo!(),
                Command::Namespace => todo!(),
                Command::Status => todo!(),
                Command::Append => todo!(),
                Command::Idle => todo!(),
                Command::Close => todo!(),
                Command::Unselect => todo!(),
                Command::Expunge(_) => todo!(),
                Command::Search(_) => todo!(),
                Command::Fetch(_) => todo!(),
                Command::Store(_) => todo!(),
                Command::Copy(_) => todo!(),
                Command::Move(_) => todo!(),
                Command::Lsub => todo!(),
                Command::Check => todo!(),
                Command::Sort(_) => todo!(),
                Command::Thread(_) => todo!(),
            }
        }

        if let Some(needs_literal) = needs_literal {
            self.write_bytes(format!("+ Ready for {} bytes.\r\n", needs_literal).into_bytes())
                .await?;
        }

        Ok(None)
    }
}

impl Request {
    pub fn is_allowed(self, state: &State, is_tls: bool) -> Result<Self, StatusResponse> {
        match &self.command {
            Command::Capability | Command::Noop | Command::Logout => Ok(self),
            Command::StartTls => {
                if !is_tls {
                    Ok(self)
                } else {
                    Err(StatusResponse::no(
                        self.tag.into(),
                        None,
                        "Already in TLS mode.",
                    ))
                }
            }
            Command::Authenticate => {
                if let State::NotAuthenticated { .. } = state {
                    Ok(self)
                } else {
                    Err(StatusResponse::no(
                        self.tag.into(),
                        None,
                        "Already authenticated.",
                    ))
                }
            }
            Command::Login => {
                if let State::NotAuthenticated { .. } = state {
                    if is_tls {
                        Ok(self)
                    } else {
                        Err(StatusResponse::no(
                            self.tag.into(),
                            None,
                            "LOGIN is disabled on the clear-text port.",
                        ))
                    }
                } else {
                    Err(StatusResponse::no(
                        self.tag.into(),
                        None,
                        "Already authenticated.",
                    ))
                }
            }
            Command::Enable
            | Command::Select
            | Command::Examine
            | Command::Create
            | Command::Delete
            | Command::Rename
            | Command::Subscribe
            | Command::Unsubscribe
            | Command::List
            | Command::Namespace
            | Command::Status
            | Command::Append
            | Command::Idle => {
                if let State::Authenticated { .. } | State::Selected { .. } = state {
                    Ok(self)
                } else {
                    Err(StatusResponse::no(
                        self.tag.into(),
                        None,
                        "Not authenticated.",
                    ))
                }
            }
            Command::Close
            | Command::Unselect
            | Command::Expunge(_)
            | Command::Search(_)
            | Command::Fetch(_)
            | Command::Store(_)
            | Command::Copy(_)
            | Command::Move(_)
            | Command::Lsub
            | Command::Check
            | Command::Sort(_)
            | Command::Thread(_) => {
                if let State::Selected { .. } = state {
                    Ok(self)
                } else {
                    Err(StatusResponse::no(
                        self.tag.into(),
                        None,
                        "No mailbox is selected.",
                    ))
                }
            }
        }
    }
}

impl State {
    pub fn auth_failures(&self) -> u8 {
        match self {
            State::NotAuthenticated { auth_failures } => *auth_failures,
            _ => unreachable!(),
        }
    }

    pub fn session_data(&self) -> Arc<SessionData> {
        match self {
            State::Authenticated { data } => data.clone(),
            State::Selected { data } => data.clone(),
            _ => unreachable!(),
        }
    }
}
