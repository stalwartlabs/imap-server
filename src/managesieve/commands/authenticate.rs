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

use jmap_client::client::Client;
use tracing::debug;

use crate::{
    commands::authenticate::{decode_challenge_oauth, decode_challenge_plain},
    core::receiver::{self, Request},
    managesieve::{
        client::{Session, State},
        Command, StatusResponse,
    },
    protocol::authenticate::Mechanism,
};

impl Session {
    pub async fn handle_authenticate(
        &mut self,
        request: Request<Command>,
    ) -> Result<bool, StatusResponse> {
        if request.tokens.is_empty() {
            return Err(StatusResponse::no("Authentication mechanism missing."));
        }

        let mut tokens = request.tokens.into_iter();
        let mechanism =
            Mechanism::parse(&tokens.next().unwrap().unwrap_bytes()).map_err(StatusResponse::no)?;
        let mut params: Vec<String> = tokens
            .into_iter()
            .filter_map(|token| token.unwrap_string().ok())
            .collect();

        let credentials = match mechanism {
            Mechanism::Plain | Mechanism::OAuthBearer => {
                if !params.is_empty() {
                    let challenge = base64::decode(&params.pop().unwrap())
                        .map_err(|_| StatusResponse::no("Failed to decode challenge."))?;
                    (if mechanism == Mechanism::Plain {
                        decode_challenge_plain(&challenge)
                    } else {
                        decode_challenge_oauth(&challenge)
                    }
                    .map_err(StatusResponse::no))?
                } else {
                    self.receiver.request = receiver::Request {
                        tag: String::new(),
                        command: Command::Authenticate,
                        tokens: vec![receiver::Token::Argument(mechanism.into_bytes())],
                    };
                    self.receiver.state = receiver::State::Argument { last_ch: b' ' };
                    return Ok(self.write_bytes(b"{0}\r\n".to_vec()).await.is_ok());
                }
            }
            _ => {
                return Err(StatusResponse::no(
                    "Authentication mechanism not supported.",
                ))
            }
        };

        match Client::new()
            .follow_redirects(&self.core.trusted_hosts)
            .forwarded_for(self.peer_addr.ip())
            .credentials(credentials)
            .connect(&self.core.jmap_url)
            .await
        {
            Ok(client) => {
                // Verify the remote JMAP server supports JMAP for Sieve.
                if client.session().sieve_capabilities().is_some() {
                    // Create session
                    self.state = State::Authenticated { client };

                    Ok(self
                        .write_bytes(StatusResponse::ok("Authentication successful").into_bytes())
                        .await
                        .is_ok())
                } else {
                    self.write_bytes(
                        StatusResponse::bye("JMAP over Sieve is not supported by the JMAP server.")
                            .into_bytes(),
                    )
                    .await
                    .ok();
                    Ok(false)
                }
            }
            Err(err) => {
                debug!("Failed to connect to {}: {}", self.core.jmap_url, err,);
                if let State::NotAuthenticated { auth_failures } = &mut self.state {
                    if *auth_failures < 3 {
                        *auth_failures += 1;
                        Err(StatusResponse::no("Authentication failed"))
                    } else {
                        self.write_bytes(
                            StatusResponse::bye("Too many authentication failures").into_bytes(),
                        )
                        .await
                        .ok();
                        debug!(
                            "Too many authentication failures, disconnecting {}",
                            self.peer_addr
                        );
                        Ok(false)
                    }
                } else {
                    unreachable!()
                }
            }
        }
    }

    pub async fn handle_unauthenticate(&mut self) -> Result<bool, StatusResponse> {
        self.state = State::NotAuthenticated { auth_failures: 0 };

        Ok(self
            .write_bytes(StatusResponse::ok("Unauthenticate successful.").into_bytes())
            .await
            .is_ok())
    }
}
