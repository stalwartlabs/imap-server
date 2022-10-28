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

use jmap_client::client::Client;
use tokio::{io::WriteHalf, net::TcpStream, sync::mpsc};
use tokio_rustls::server::TlsStream;
use tracing::debug;

use crate::core::{
    receiver::{self, Receiver, Request},
    writer::{self, Event},
    Core,
};

use super::{Command, ResponseCode, StatusResponse};

pub struct Session {
    pub core: Arc<Core>,
    pub receiver: Receiver<Command>,
    pub state: State,
    pub peer_addr: SocketAddr,
    pub is_tls: bool,
    pub writer: mpsc::Sender<writer::Event>,
}

#[allow(clippy::large_enum_variant)]
pub enum State {
    NotAuthenticated {
        auth_failures: u8,
    },
    Authenticated {
        client: Client,
        core: Arc<Core>,
        writer: mpsc::Sender<writer::Event>,
    },
}

impl Session {
    pub fn new(core: Arc<Core>, peer_addr: SocketAddr, is_tls: bool) -> Self {
        Session {
            receiver: Receiver::with_max_request_size(core.max_request_size)
                .with_start_state(receiver::State::Command { is_uid: false }),
            state: State::NotAuthenticated { auth_failures: 0 },
            peer_addr,
            is_tls,
            writer: writer::spawn_writer(),
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
                    self.write_bytes(StatusResponse::no(response.message).into_bytes())
                        .await?;
                    break;
                }
            }
        }

        for request in requests {
            match request.command {
                Command::Authenticate => todo!(),
                Command::StartTls => todo!(),
                Command::Logout => todo!(),
                Command::Capability => todo!(),
                Command::HaveSpace => todo!(),
                Command::PutScript => todo!(),
                Command::ListScripts => todo!(),
                Command::SetActive => todo!(),
                Command::GetScript => todo!(),
                Command::DeleteScript => todo!(),
                Command::RenameScript => todo!(),
                Command::CheckScript => todo!(),
                Command::Noop => todo!(),
                Command::Unauthenticate => todo!(),
            }
        }

        if let Some(needs_literal) = needs_literal {
            self.write_bytes(format!("OK Ready for {} bytes.\r\n", needs_literal).into_bytes())
                .await?;
        }

        Ok(None)
    }

    pub async fn write_bytes(&self, bytes: Vec<u8>) -> Result<(), ()> {
        if let Err(err) = self.writer.send(Event::Bytes(bytes)).await {
            debug!("Failed to send bytes: {}", err);
            Err(())
        } else {
            Ok(())
        }
    }
}

impl Request<Command> {
    pub fn is_allowed(self, state: &State, is_tls: bool) -> Result<Self, StatusResponse> {
        match &self.command {
            Command::Capability | Command::Logout | Command::Noop => Ok(self),
            Command::Authenticate => {
                if let State::NotAuthenticated { .. } = state {
                    if is_tls {
                        Ok(self)
                    } else {
                        Err(StatusResponse::no("Cannot authenticate over plain-text.")
                            .with_code(ResponseCode::EncryptNeeded))
                    }
                } else {
                    Err(StatusResponse::no("Already authenticated."))
                }
            }
            Command::StartTls => {
                if !is_tls {
                    Ok(self)
                } else {
                    Err(StatusResponse::no("Already in TLS mode."))
                }
            }
            Command::HaveSpace
            | Command::PutScript
            | Command::ListScripts
            | Command::SetActive
            | Command::GetScript
            | Command::DeleteScript
            | Command::RenameScript
            | Command::CheckScript
            | Command::Unauthenticate => {
                if let State::Authenticated { .. } = state {
                    Ok(self)
                } else {
                    Err(StatusResponse::no("Not authenticated."))
                }
            }
        }
    }
}
