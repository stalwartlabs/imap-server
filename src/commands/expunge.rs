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

use std::sync::Arc;

use jmap_client::{core::query, email::query::Filter};

use crate::{
    core::{
        client::{SelectedMailbox, Session, SessionData},
        receiver::{Request, Token},
        Command, Flag, IntoStatusResponse, ResponseCode, StatusResponse,
    },
    parser::parse_sequence_set,
    protocol::{expunge::Response, select::Exists, Sequence},
};

use super::search::SavedSearch;

impl Session {
    pub async fn handle_expunge(&mut self, request: Request, is_uid: bool) -> Result<(), ()> {
        let (data, mailbox) = self.state.select_data();

        // Parse sequence to operate on
        let sequence = if let Some(Token::Argument(value)) = request.tokens.into_iter().next() {
            parse_sequence_set(&value).ok()
        } else {
            None
        };

        let jmap_state = match data.expunge(mailbox.clone(), sequence).await {
            Ok(jmap_state) => jmap_state,
            Err(response) => {
                return self
                    .write_bytes(response.with_tag(request.tag).into_bytes())
                    .await;
            }
        };

        // Remove saved searches
        *mailbox.saved_search.lock() = SavedSearch::None;

        match data.synchronize_messages(mailbox.id.clone()).await {
            Ok(mut new_state) => {
                let mut buf = Vec::with_capacity(64);

                {
                    let mut deleted_ids = Vec::new();
                    let mut state = mailbox.state.lock();

                    for (seqnum, uid) in state.imap_uids.iter().enumerate() {
                        if !new_state.imap_uids.contains(uid) {
                            deleted_ids.push(if self.is_qresync {
                                *uid
                            } else {
                                (seqnum + 1) as u32
                            });
                        }
                    }

                    if !deleted_ids.is_empty() || state.total_messages != new_state.total_messages {
                        if !deleted_ids.is_empty() {
                            deleted_ids.sort_unstable();
                            Response {
                                is_qresync: self.is_qresync,
                                ids: deleted_ids,
                            }
                            .serialize_to(&mut buf);
                        }
                        Exists {
                            total_messages: new_state.total_messages,
                        }
                        .serialize(&mut buf);
                    }

                    new_state.last_state = jmap_state;
                    *state = new_state;
                }

                self.write_bytes(
                    StatusResponse::completed(Command::Expunge(is_uid))
                        .with_tag(request.tag)
                        .serialize(buf),
                )
                .await
            }
            Err(response) => {
                self.write_bytes(response.with_tag(request.tag).into_bytes())
                    .await
            }
        }
    }
}

impl SessionData {
    pub async fn expunge(
        &self,
        mailbox: Arc<SelectedMailbox>,
        sequence: Option<Sequence>,
    ) -> crate::core::Result<String> {
        let mut request = self.client.build();
        let result_ref = request
            .query_email()
            .account_id(&mailbox.id.account_id)
            .filter(query::Filter::and({
                let mut filters = vec![Filter::has_keyword(Flag::Deleted.to_jmap())];

                if let Some(mailbox_id) = &mailbox.id.mailbox_id {
                    filters.push(Filter::in_mailbox(mailbox_id));
                }

                if let Some(sequence) = sequence {
                    filters.push(Filter::id(
                        mailbox
                            .sequence_to_jmap(&sequence, true)
                            .await?
                            .into_iter()
                            .map(|(k, _)| k),
                    ));
                }

                filters
            }))
            .result_reference();
        request
            .set_email()
            .account_id(&mailbox.id.account_id)
            .destroy_ref(result_ref);
        let mut response = request
            .send()
            .await
            .map_err(|err| err.into_status_response())?
            .unwrap_method_responses();
        if response.len() != 2 {
            return Err(StatusResponse::no("Invalid JMAP server response")
                .with_code(ResponseCode::ContactAdmin));
        }

        Ok(response
            .pop()
            .unwrap()
            .unwrap_set_email()
            .map_err(|err| err.into_status_response())?
            .take_new_state())
    }
}
