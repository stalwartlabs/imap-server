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

use std::{iter::Peekable, net::SocketAddr, sync::Arc, vec::IntoIter};

use jmap_client::client::Client;
use tokio::{
    io::WriteHalf,
    net::TcpStream,
    sync::{mpsc, watch},
};
use tokio_rustls::server::TlsStream;
use tracing::debug;

use crate::{commands::search::SavedSearch, protocol::ProtocolVersion};

use super::{
    mailbox::Account,
    message::{MailboxData, MailboxId},
    receiver::{self, Receiver, Request},
    writer, Command, Core, StatusResponse,
};

pub struct Session {
    pub core: Arc<Core>,
    pub receiver: Receiver<Command>,
    pub version: ProtocolVersion,
    pub state: State,
    pub peer_addr: SocketAddr,
    pub is_tls: bool,
    pub is_condstore: bool,
    pub is_qresync: bool,
    pub writer: mpsc::Sender<writer::Event>,
    pub idle_tx: Option<watch::Sender<bool>>,
}

pub struct SessionData {
    pub client: Client,
    pub core: Arc<Core>,
    pub writer: mpsc::Sender<writer::Event>,
    pub mailboxes: parking_lot::Mutex<Vec<Account>>,
}

pub struct SelectedMailbox {
    pub id: Arc<MailboxId>,
    pub state: parking_lot::Mutex<MailboxData>,
    pub saved_search: parking_lot::Mutex<SavedSearch>,
    pub is_select: bool,
    pub is_condstore: bool,
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
        mailbox: Arc<SelectedMailbox>,
    },
}

impl Session {
    pub fn new(core: Arc<Core>, peer_addr: SocketAddr, is_tls: bool) -> Self {
        Session {
            receiver: Receiver::with_max_request_size(core.max_request_size),
            version: ProtocolVersion::Rev1,
            state: State::NotAuthenticated { auth_failures: 0 },
            peer_addr,
            is_tls,
            writer: writer::spawn_writer(),
            idle_tx: None,
            is_condstore: false,
            is_qresync: false,
            core,
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
        /*let tmp = "dd";
        for line in String::from_utf8_lossy(bytes).split("\r\n") {
            if let Some((tag, _)) = line.split_once(' ') {
                if tag.len() < 10 && tag.contains('.') {
                    println!("<- {:?}", &line[..std::cmp::min(line.len(), 100)]);
                }
            }
        }*/

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

        let mut requests = requests.into_iter().peekable();
        while let Some(request) = requests.next() {
            match request.command {
                Command::List | Command::Lsub => {
                    self.handle_list(request).await?;
                }
                Command::Select | Command::Examine => {
                    self.handle_select(request).await?;
                }
                Command::Create => {
                    self.handle_create(group_requests(&mut requests, vec![request]))
                        .await?;
                }
                Command::Delete => {
                    self.handle_delete(group_requests(&mut requests, vec![request]))
                        .await?;
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
                Command::Search(is_uid) => {
                    self.handle_search(request, false, is_uid).await?;
                }
                Command::Fetch(is_uid) => {
                    self.handle_fetch(request, is_uid).await?;
                }
                Command::Store(is_uid) => {
                    self.handle_store(request, is_uid).await?;
                }
                Command::Copy(is_uid) => {
                    self.handle_copy_move(request, false, is_uid).await?;
                }
                Command::Move(is_uid) => {
                    self.handle_copy_move(request, true, is_uid).await?;
                }
                Command::Sort(is_uid) => {
                    self.handle_search(request, true, is_uid).await?;
                }
                Command::Thread(is_uid) => {
                    self.handle_thread(request, is_uid).await?;
                }
                Command::Idle => {
                    self.handle_idle(request).await?;
                }
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
                Command::SetAcl => {
                    self.handle_set_acl(request).await?;
                }
                Command::DeleteAcl => {
                    self.handle_delete_acl(request).await?;
                }
                Command::GetAcl => {
                    self.handle_get_acl(request).await?;
                }
                Command::ListRights => {
                    self.handle_list_rights(request).await?;
                }
                Command::MyRights => {
                    self.handle_my_rights(request).await?;
                }
                Command::Unauthenticate => {
                    self.handle_unauthenticate(request).await?;
                }
                Command::Id => {
                    self.handle_id(request).await?;
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

pub fn group_requests(
    requests: &mut Peekable<IntoIter<Request<Command>>>,
    mut grouped_requests: Vec<Request<Command>>,
) -> Vec<Request<Command>> {
    let last_command = grouped_requests.last().unwrap().command;
    loop {
        match requests.peek() {
            Some(request) if request.command == last_command => {
                grouped_requests.push(requests.next().unwrap());
            }
            _ => break,
        }
    }
    grouped_requests
}

impl Request<Command> {
    pub fn is_allowed(self, state: &State, is_tls: bool) -> Result<Self, StatusResponse> {
        match &self.command {
            Command::Capability | Command::Noop | Command::Logout | Command::Id => Ok(self),
            Command::StartTls => {
                if !is_tls {
                    Ok(self)
                } else {
                    Err(StatusResponse::no("Already in TLS mode.").with_tag(self.tag))
                }
            }
            Command::Authenticate => {
                if let State::NotAuthenticated { .. } = state {
                    Ok(self)
                } else {
                    Err(StatusResponse::no("Already authenticated.").with_tag(self.tag))
                }
            }
            Command::Login => {
                if let State::NotAuthenticated { .. } = state {
                    if is_tls {
                        Ok(self)
                    } else {
                        Err(
                            StatusResponse::no("LOGIN is disabled on the clear-text port.")
                                .with_tag(self.tag),
                        )
                    }
                } else {
                    Err(StatusResponse::no("Already authenticated.").with_tag(self.tag))
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
            | Command::Idle
            | Command::SetAcl
            | Command::DeleteAcl
            | Command::GetAcl
            | Command::ListRights
            | Command::MyRights
            | Command::Unauthenticate => {
                if let State::Authenticated { .. } | State::Selected { .. } = state {
                    Ok(self)
                } else {
                    Err(StatusResponse::no("Not authenticated.").with_tag(self.tag))
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
                State::Selected { mailbox, .. } => {
                    if mailbox.is_select
                        || !matches!(
                            self.command,
                            Command::Store(_) | Command::Expunge(_) | Command::Move(_),
                        )
                    {
                        Ok(self)
                    } else {
                        Err(StatusResponse::no("Not permitted in EXAMINE state.")
                            .with_tag(self.tag))
                    }
                }
                State::Authenticated { .. } => {
                    Err(StatusResponse::bad("No mailbox is selected.").with_tag(self.tag))
                }
                State::NotAuthenticated { .. } => {
                    Err(StatusResponse::no("Not authenticated.").with_tag(self.tag))
                }
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

    pub fn mailbox_data(&self) -> (Arc<SessionData>, Arc<SelectedMailbox>) {
        match self {
            State::Selected { data, mailbox, .. } => (data.clone(), mailbox.clone()),
            _ => unreachable!(),
        }
    }

    pub fn session_mailbox_data(&self) -> (Arc<SessionData>, Option<Arc<SelectedMailbox>>) {
        match self {
            State::Authenticated { data } => (data.clone(), None),
            State::Selected { data, mailbox, .. } => (data.clone(), mailbox.clone().into()),
            _ => unreachable!(),
        }
    }

    pub fn select_data(&self) -> (Arc<SessionData>, Arc<SelectedMailbox>) {
        match self {
            State::Selected { data, mailbox } => (data.clone(), mailbox.clone()),
            _ => unreachable!(),
        }
    }

    pub fn is_authenticated(&self) -> bool {
        matches!(self, State::Authenticated { .. } | State::Selected { .. })
    }

    pub fn is_mailbox_selected(&self) -> bool {
        matches!(self, State::Selected { .. })
    }
}
