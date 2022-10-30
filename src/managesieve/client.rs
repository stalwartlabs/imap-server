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

use jmap_client::{client::Client, sieve::query::Filter};
use tokio::{io::WriteHalf, net::TcpStream, sync::mpsc};
use tokio_rustls::server::TlsStream;
use tracing::debug;

use crate::core::{
    receiver::{self, Receiver, Request},
    writer::{self, Event},
    Core,
};

use super::{commands::IntoStatusResponse, Command, ResponseCode, StatusResponse};

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
    NotAuthenticated { auth_failures: u8 },
    Authenticated { client: Client },
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
        /*let tmp = "dd";
        for line in String::from_utf8_lossy(bytes).split("\r\n") {
            println!("<- {:?}", &line[..std::cmp::min(line.len(), 100)]);
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
                    self.write_bytes(StatusResponse::no(response.message).into_bytes())
                        .await?;
                    break;
                }
            }
        }

        for request in requests {
            let result = match request.command {
                Command::ListScripts => self.handle_listscripts().await,
                Command::PutScript => self.handle_putscript(request).await,
                Command::SetActive => self.handle_setactive(request).await,
                Command::GetScript => self.handle_getscript(request).await,
                Command::DeleteScript => self.handle_deletescript(request).await,
                Command::RenameScript => self.handle_renamescript(request).await,
                Command::CheckScript => self.handle_checkscript(request).await,
                Command::HaveSpace => self.handle_havespace(request).await,
                Command::Capability => self.handle_capability("").await,
                Command::Authenticate => self.handle_authenticate(request).await,
                Command::StartTls => return self.handle_starttls().await,
                Command::Logout => self.handle_logout().await,
                Command::Noop => self.handle_noop(request).await,
                Command::Unauthenticate => self.handle_unauthenticate().await,
            };

            match result {
                Ok(true) => (),
                Ok(false) => return Err(()),
                Err(response) => {
                    self.write_bytes(response.into_bytes()).await?;
                }
            }
        }

        if let Some(needs_literal) = needs_literal {
            self.write_bytes(format!("OK Ready for {} bytes.\r\n", needs_literal).into_bytes())
                .await?;
        }

        Ok(None)
    }

    pub async fn write_bytes(&self, bytes: Vec<u8>) -> Result<(), ()> {
        /*let tmp = "dd";
        println!(
            "-> {:?}",
            String::from_utf8_lossy(&bytes[..std::cmp::min(bytes.len(), 100)])
        );*/

        if let Err(err) = self.writer.send(Event::Bytes(bytes)).await {
            debug!("Failed to send bytes: {}", err);
            Err(())
        } else {
            Ok(())
        }
    }

    pub fn client(&self) -> &Client {
        if let State::Authenticated { client } = &self.state {
            client
        } else {
            unreachable!()
        }
    }

    pub async fn get_script_id(&self, name: String) -> Result<String, StatusResponse> {
        self.client()
            .sieve_script_query(Filter::name(name).into(), None::<Vec<_>>)
            .await
            .map_err(|err| err.into_status_response())?
            .take_ids()
            .pop()
            .ok_or_else(|| {
                StatusResponse::no("There is no script by that name")
                    .with_code(ResponseCode::NonExistent)
            })
    }
}

impl Request<Command> {
    pub fn is_allowed(self, state: &State, is_tls: bool) -> Result<Self, StatusResponse> {
        match &self.command {
            Command::Capability | Command::Logout | Command::Noop => Ok(self),
            Command::Authenticate => {
                if let State::NotAuthenticated { .. } = state {
                    #[cfg(test)]
                    {
                        Ok(self)
                    }

                    #[cfg(not(test))]
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
