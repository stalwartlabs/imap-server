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

use jmap_client::core::response::MethodResponse;
use tracing::debug;

use crate::{
    core::{
        client::{SelectedMailbox, Session, SessionData},
        message::{MailboxId, MappingOptions},
        receiver::Request,
        Command, IntoStatusResponse, ResponseCode, StatusResponse,
    },
    protocol::{copy_move::Arguments, expunge, ImapResponse},
};

impl Session {
    pub async fn handle_copy_move(
        &mut self,
        request: Request,
        is_move: bool,
        is_uid: bool,
    ) -> Result<(), ()> {
        match request.parse_copy_move() {
            Ok(arguments) => {
                let (data, src_mailbox) = self.state.mailbox_data();

                // Refresh mailboxes
                if let Err(err) = data.synchronize_mailboxes(false, false).await {
                    debug!("Failed to refresh mailboxes: {}", err);
                    return self
                        .write_bytes(
                            err.into_status_response()
                                .with_tag(arguments.tag)
                                .into_bytes(),
                        )
                        .await;
                }

                // Make sure the mailbox exists.
                let dest_mailbox =
                    if let Some(mailbox) = data.get_mailbox_by_name(&arguments.mailbox_name) {
                        if mailbox.mailbox_id.is_some() {
                            Arc::new(mailbox)
                        } else {
                            return self
                                .write_bytes(
                                    StatusResponse::no(
                                        "Appending messages to this mailbox is not allowed.",
                                    )
                                    .with_tag(arguments.tag)
                                    .with_code(ResponseCode::Cannot)
                                    .into_bytes(),
                                )
                                .await;
                        }
                    } else {
                        return self
                            .write_bytes(
                                StatusResponse::no("Destination mailbox does not exist.")
                                    .with_tag(arguments.tag)
                                    .with_code(ResponseCode::TryCreate)
                                    .into_bytes(),
                            )
                            .await;
                    };

                // Check that the destination mailbox is not the same as the source mailbox.
                if src_mailbox.id.account_id == dest_mailbox.account_id
                    && src_mailbox.id.mailbox_id == dest_mailbox.mailbox_id
                {
                    return self
                        .write_bytes(
                            StatusResponse::no("Source and destination mailboxes are the same.")
                                .with_tag(arguments.tag)
                                .with_code(ResponseCode::Cannot)
                                .into_bytes(),
                        )
                        .await;
                }

                let is_qresync = self.is_qresync;
                tokio::spawn(async move {
                    if let Err(err) = data
                        .copy_move(
                            arguments,
                            src_mailbox,
                            dest_mailbox,
                            is_move,
                            is_uid,
                            is_qresync,
                        )
                        .await
                    {
                        data.write_bytes(err.into_bytes()).await;
                    }
                });
                Ok(())
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}

impl SessionData {
    pub async fn copy_move(
        &self,
        arguments: Arguments,
        src_mailbox: Arc<SelectedMailbox>,
        dest_mailbox: Arc<MailboxId>,
        is_move: bool,
        is_uid: bool,
        is_qresync: bool,
    ) -> Result<(), StatusResponse> {
        // Convert IMAP ids to JMAP ids.
        let ids = match src_mailbox
            .sequence_to_jmap(&arguments.sequence_set, is_uid)
            .await
        {
            Ok(ids) => {
                if ids.is_empty() {
                    return Err(
                        StatusResponse::no("No messages were found.").with_tag(arguments.tag)
                    );
                }
                ids
            }
            Err(response) => {
                return Err(response.with_tag(arguments.tag));
            }
        };

        let response = StatusResponse::completed(if is_move {
            Command::Move(is_uid)
        } else {
            Command::Copy(is_uid)
        });

        let max_objects_in_set = self
            .client
            .session()
            .core_capabilities()
            .map(|c| c.max_objects_in_set())
            .unwrap_or(500);

        let mut src_uids = Vec::with_capacity(ids.len());
        let (mut copied_ids, destroyed_ids) =
            if src_mailbox.id.account_id == dest_mailbox.account_id {
                // Mailboxes are in the same account, send a Email/set request.
                let mut request = self.client.build();
                let ids_vec = ids.keys().collect::<Vec<_>>();
                for jmap_ids in ids_vec.chunks(max_objects_in_set) {
                    let set_request = request.set_email().account_id(&src_mailbox.id.account_id);
                    for &jmap_id in jmap_ids {
                        let update_item = set_request.update(jmap_id);
                        update_item.mailbox_id(dest_mailbox.mailbox_id.as_ref().unwrap(), true);
                        if is_move {
                            if let Some(mailbox_id) = &src_mailbox.id.mailbox_id {
                                update_item.mailbox_id(mailbox_id, false);
                            }
                        }
                    }
                }
                let mut copied_ids = Vec::with_capacity(ids.len());
                for response in request
                    .send()
                    .await
                    .map_err(|err| {
                        err.into_status_response()
                            .with_tag(arguments.tag.to_string())
                    })?
                    .unwrap_method_responses()
                {
                    let mut response = response.unwrap_set_email().map_err(|err| {
                        err.into_status_response()
                            .with_tag(arguments.tag.to_string())
                    })?;

                    // Update last state
                    if is_move {
                        src_mailbox.state.lock().last_state = response.take_new_state();
                    }

                    if let Some(updated_ids) = response.take_updated_ids() {
                        for updated_id in updated_ids {
                            if let Some(imap_id) = ids.get(&updated_id) {
                                src_uids.push(imap_id.uid);
                            }
                            copied_ids.push(updated_id);
                        }
                    }
                }
                (copied_ids, None)
            } else {
                // Mailboxes are in different accounts, send a Email/copy request.
                let mut request = self.client.build();

                let ids_vec = ids.keys().collect::<Vec<_>>();
                for jmap_ids in ids_vec.chunks(max_objects_in_set) {
                    let copy_request = request
                        .copy_email(&src_mailbox.id.account_id)
                        .account_id(&dest_mailbox.account_id)
                        .on_success_destroy_original(is_move);
                    for &jmap_id in jmap_ids {
                        copy_request
                            .create(jmap_id)
                            .mailbox_id(dest_mailbox.mailbox_id.as_ref().unwrap(), true);
                    }
                }
                let mut copied_ids = Vec::with_capacity(ids.len());
                let mut destroyed_ids = Vec::new();

                for response in request
                    .send()
                    .await
                    .map_err(|err| {
                        err.into_status_response()
                            .with_tag(arguments.tag.to_string())
                    })?
                    .unwrap_method_responses()
                {
                    match response.unwrap_method_response() {
                        MethodResponse::CopyEmail(mut response) => {
                            if let Some(updated_emails) = response.take_created() {
                                for mut updated_email in updated_emails {
                                    let updated_id = updated_email.take_id();
                                    if let Some(imap_id) = ids.get(&updated_id) {
                                        src_uids.push(imap_id.uid);
                                    }
                                    copied_ids.push(updated_id);
                                }
                            }
                        }
                        MethodResponse::SetEmail(mut response) => {
                            src_mailbox.state.lock().last_state = response.take_new_state();
                            if let Some(destroyed_ids_) = response.take_destroyed_ids() {
                                destroyed_ids.extend(destroyed_ids_);
                            }
                        }
                        MethodResponse::Error(err) => {
                            return Err(jmap_client::Error::from(err)
                                .into_status_response()
                                .with_tag(arguments.tag));
                        }
                        _ => (),
                    }
                }

                (copied_ids, destroyed_ids.into())
            };

        if copied_ids.is_empty() {
            return Err(StatusResponse::no("Copy failed.").with_tag(arguments.tag));
        }

        // Map copied JMAP Ids to IMAP UIDs in the destination folder.
        let uid_copy = if let (Ok((copied_ids_, mut dest_uids)), Ok((uid_validity, _))) = (
            self.core
                .jmap_to_imap(
                    dest_mailbox.clone(),
                    copied_ids,
                    MappingOptions::AddIfMissing,
                )
                .await,
            self.core.uids(dest_mailbox).await,
        ) {
            copied_ids = copied_ids_;
            src_uids.sort_unstable();
            dest_uids.sort_unstable();
            ResponseCode::CopyUid {
                uid_validity,
                src_uids,
                dest_uids,
            }
        } else {
            return Err(StatusResponse::database_failure().with_tag(arguments.tag));
        };

        // Remove UIDS on move
        let bytes = if is_move {
            let destroyed_ids = if let Some(destroyed_ids) = destroyed_ids {
                destroyed_ids
            } else {
                copied_ids
            };
            let mut expunged_ids = Vec::with_capacity(destroyed_ids.len());
            {
                let mut state = src_mailbox.state.lock();
                let mut new_jmap_ids = Vec::with_capacity(state.jmap_ids.len());
                let mut new_imap_uids = Vec::with_capacity(state.imap_uids.len());

                for (pos, (jmap_id, imap_uid)) in std::mem::take(&mut state.jmap_ids)
                    .into_iter()
                    .zip(std::mem::take(&mut state.imap_uids))
                    .enumerate()
                {
                    if !destroyed_ids.contains(&jmap_id) {
                        new_jmap_ids.push(jmap_id);
                        new_imap_uids.push(imap_uid);
                    } else {
                        expunged_ids.push(if is_qresync {
                            imap_uid
                        } else {
                            (pos + 1) as u32
                        });
                    }
                }

                state.total_messages = state.total_messages.saturating_sub(expunged_ids.len());
                state.jmap_ids = new_jmap_ids;
                state.imap_uids = new_imap_uids;

                expunged_ids.sort_unstable();
            }

            self.core
                .delete_ids(src_mailbox.id.clone(), destroyed_ids)
                .await
                .ok();

            response.with_tag(arguments.tag).serialize(
                StatusResponse::ok("Copied UIDs")
                    .with_code(uid_copy)
                    .serialize(
                        expunge::Response {
                            is_qresync,
                            ids: expunged_ids,
                        }
                        .serialize(),
                    ),
            )
        } else {
            response
                .with_tag(arguments.tag)
                .with_code(uid_copy)
                .into_bytes()
        };

        self.write_bytes(bytes).await;

        Ok(())
    }
}
