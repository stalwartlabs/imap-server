use std::{net::SocketAddr, sync::Arc};

use jmap_client::client::Client;
use tokio::{io::WriteHalf, net::TcpStream, sync::mpsc};
use tokio_rustls::server::TlsStream;
use tracing::debug;

use crate::protocol::ProtocolVersion;

use super::{
    mailbox::Account,
    message::MailboxData,
    receiver::{self, Receiver, Request},
    writer, Command, Core, StatusResponse,
};

pub struct Session {
    pub core: Arc<Core>,
    pub receiver: Receiver,
    pub version: ProtocolVersion,
    pub state: State,
    pub peer_addr: SocketAddr,
    pub is_tls: bool,
    pub writer: mpsc::Sender<writer::Event>,
}

pub struct SessionData {
    pub client: Client,
    pub core: Arc<Core>,
    pub writer: mpsc::Sender<writer::Event>,
    pub mailboxes: parking_lot::Mutex<Vec<Account>>,
}

pub enum State {
    NotAuthenticated {
        auth_failures: u8,
    },
    Authenticated {
        data: Arc<SessionData>,
    },
    Selected {
        data: Arc<SessionData>,
        mailbox: Arc<MailboxData>,
        rw: bool,
    },
}

impl Session {
    pub fn new(core: Arc<Core>, peer_addr: SocketAddr, is_tls: bool) -> Self {
        Session {
            core,
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
                Command::List | Command::Lsub => {
                    self.handle_list(request).await?;
                }
                Command::Select | Command::Examine => {
                    self.handle_select(request).await?;
                }
                Command::Create => {
                    self.handle_create(request).await?;
                }
                Command::Delete => {
                    self.handle_delete(request).await?;
                }
                Command::Rename => {
                    self.handle_rename(request).await?;
                }
                Command::Status => {
                    self.handle_status(request).await?;
                }
                Command::Append => {
                    self.handle_append(request).await?;
                }
                Command::Close => {
                    self.handle_close(request).await?;
                }
                Command::Unselect => {
                    self.handle_unselect(request).await?;
                }
                Command::Expunge(is_uid) => {
                    self.handle_expunge(request, is_uid).await?;
                }
                Command::Search(_) => todo!(),
                Command::Fetch(_) => todo!(),
                Command::Store(is_uid) => {
                    self.handle_store(request, is_uid).await?;
                }
                Command::Copy(_) => todo!(),
                Command::Move(_) => todo!(),
                Command::Sort(_) => todo!(),
                Command::Thread(_) => todo!(),
                Command::Idle => todo!(), //TODO
                Command::Subscribe => {
                    self.handle_subscribe(request, true).await?;
                }
                Command::Unsubscribe => {
                    self.handle_subscribe(request, false).await?;
                }
                Command::Namespace => {
                    self.handle_namespace(request).await?;
                }
                Command::Authenticate => {
                    self.handle_authenticate(request).await?;
                }
                Command::Login => {
                    self.handle_login(request).await?;
                }
                Command::Capability => {
                    self.handle_capability(request).await?;
                }
                Command::Enable => {
                    self.handle_enable(request).await?;
                }
                Command::StartTls => {
                    return self.handle_starttls(request).await;
                }
                Command::Noop => {
                    self.handle_noop(request, false).await?;
                }
                Command::Check => {
                    self.handle_noop(request, true).await?;
                }
                Command::Logout => {
                    self.handle_logout(request).await?;
                }
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
            | Command::Lsub
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
            | Command::Check
            | Command::Sort(_)
            | Command::Thread(_) => match state {
                State::Selected { rw, .. } => {
                    if *rw
                        || !matches!(
                            self.command,
                            Command::Store(_) | Command::Expunge(_) | Command::Move(_),
                        )
                    {
                        Ok(self)
                    } else {
                        Err(StatusResponse::no(
                            self.tag.into(),
                            None,
                            "Not permitted in EXAMINE state.",
                        ))
                    }
                }
                State::Authenticated { .. } => Err(StatusResponse::bad(
                    self.tag.into(),
                    None,
                    "No mailbox is selected.",
                )),
                State::NotAuthenticated { .. } => Err(StatusResponse::no(
                    self.tag.into(),
                    None,
                    "Not authenticated.",
                )),
            },
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
            State::Selected { data, .. } => data.clone(),
            _ => unreachable!(),
        }
    }

    pub fn mailbox_data(&self) -> (Arc<SessionData>, Arc<MailboxData>, bool) {
        match self {
            State::Selected { data, mailbox, rw } => (data.clone(), mailbox.clone(), *rw),
            _ => unreachable!(),
        }
    }

    pub fn is_mailbox_selected(&self) -> bool {
        matches!(self, State::Selected { .. })
    }
}
