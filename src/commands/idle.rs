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

use futures::{Stream, StreamExt};
use jmap_client::{event_source::Changes, TypeState};
use tokio::sync::watch;
use tracing::debug;

use crate::{
    core::{
        client::{SelectedMailbox, Session, SessionData, State},
        receiver::Request,
        Command, IntoStatusResponse, ResponseCode, StatusResponse,
    },
    protocol::{
        expunge, fetch,
        list::{Attribute, ListItem},
        select::Exists,
        status::Status,
        Sequence,
    },
};

impl Session {
    pub async fn handle_idle(&mut self, request: Request<Command>) -> Result<(), ()> {
        let (data, mailbox, subscriptions) = match &self.state {
            State::Authenticated { data } => (data.clone(), None, vec![TypeState::Mailbox]),
            State::Selected { data, mailbox, .. } => (
                data.clone(),
                mailbox.clone().into(),
                vec![TypeState::Email, TypeState::Mailbox],
            ),
            _ => unreachable!(),
        };

        // Start event source
        let changes = match data
            .client
            .event_source(subscriptions.into(), false, 30.into(), None)
            .await
        {
            Ok(changes) => changes,
            Err(err) => {
                debug!("Error starting event source: {}", err);
                return self
                    .write_bytes(
                        StatusResponse::no("It was not possible to start IDLE.")
                            .with_tag(request.tag)
                            .with_code(ResponseCode::ContactAdmin)
                            .into_bytes(),
                    )
                    .await;
            }
        };

        // Send continuation response
        self.write_bytes(b"+ Idling, send 'DONE' to stop.\r\n".to_vec())
            .await?;

        // Create channel
        let (idle_tx, idle_rx) = watch::channel(true);
        self.idle_tx = idle_tx.into();
        let is_rev2 = self.version.is_rev2();
        let is_qresync = self.is_qresync;

        tokio::spawn(async move {
            data.idle(mailbox, changes, idle_rx, request.tag, is_qresync, is_rev2)
                .await;
        });
        Ok(())
    }
}

impl SessionData {
    pub async fn idle(
        &self,
        mailbox: Option<Arc<SelectedMailbox>>,
        mut changes: impl Stream<Item = jmap_client::Result<Changes>> + Unpin,
        mut idle_rx: watch::Receiver<bool>,
        tag: String,
        is_qresync: bool,
        is_rev2: bool,
    ) {
        // Write any pending changes
        self.write_changes(mailbox.as_ref(), true, true, is_qresync, is_rev2)
            .await;

        loop {
            tokio::select! {
                changes = changes.next() => {
                    match changes {
                        Some(Ok(changes)) => {
                            let mut has_mailbox_changes = false;
                            let mut has_email_changes = false;
                            for (account_id, changes) in changes.into_inner() {
                                for (type_state, _) in changes {
                                    match type_state {
                                        TypeState::Mailbox => {
                                            has_mailbox_changes = true;
                                        }
                                        TypeState::Email if mailbox.as_ref().map_or(false, |m| m.id.account_id == account_id) => {
                                            has_email_changes = true;
                                        }
                                        _ => (),
                                    }
                                }
                            }

                            self.write_changes(
                                mailbox.as_ref(),
                                has_mailbox_changes,
                                has_email_changes,
                                is_qresync,
                                is_rev2
                            ).await;

                        },
                        Some(Err(err)) => {
                            debug!("EventSource error: {}", err);
                        }
                        None => {
                            debug!("EventSource connection unexpectedly closed.");
                            break;
                        },
                    }
                },
                _ = idle_rx.changed() => {
                    self.write_bytes(StatusResponse::completed(Command::Idle).with_tag(tag).into_bytes())
                        .await;
                    return;
                }
            };
        }

        // Connection was unexpectedly closed.
        // TODO: Try reconnecting.
        idle_rx.changed().await.ok();
        self.write_bytes(
            StatusResponse::completed(Command::Idle)
                .with_tag(tag)
                .into_bytes(),
        )
        .await;
    }

    pub async fn write_changes(
        &self,
        mailbox: Option<&Arc<SelectedMailbox>>,
        check_mailboxes: bool,
        check_emails: bool,
        is_qresync: bool,
        is_rev2: bool,
    ) {
        // Fetch all changed mailboxes
        if check_mailboxes {
            match self.synchronize_mailboxes(true, false).await {
                Ok(Some(changes)) => {
                    let mut buf = Vec::with_capacity(64);

                    // List deleted mailboxes
                    for mailbox_name in changes.deleted {
                        ListItem {
                            mailbox_name,
                            attributes: vec![Attribute::NonExistent],
                            tags: vec![],
                        }
                        .serialize(&mut buf, is_rev2, false);
                    }

                    // List added mailboxes
                    for mailbox_name in changes.added {
                        ListItem {
                            mailbox_name: mailbox_name.to_string(),
                            attributes: vec![],
                            tags: vec![],
                        }
                        .serialize(&mut buf, is_rev2, false);
                    }
                    // Obtain status of changed mailboxes
                    for mailbox_name in changes.changed {
                        if let Ok(status) = self
                            .status(
                                mailbox_name,
                                &[
                                    Status::Messages,
                                    Status::Unseen,
                                    Status::UidNext,
                                    Status::UidValidity,
                                ],
                            )
                            .await
                        {
                            status.serialize(&mut buf, is_rev2);
                        }
                    }

                    if !buf.is_empty() {
                        self.write_bytes(buf).await;
                    }
                }
                Err(err) => {
                    debug!("Failed to refresh mailboxes: {}", err);
                }
                _ => unreachable!(),
            }
        }

        // Fetch selected mailbox changes
        if check_emails {
            // Synchronize emails
            if let Some(mailbox) = mailbox {
                // Obtain changes since last sync
                let mut request = self.client.build();
                request
                    .changes_email(&mailbox.state.lock().last_state)
                    .account_id(&mailbox.id.account_id);
                let mut response = match request.send_changes_email().await {
                    Ok(response) => response,
                    Err(err) => {
                        debug!("Failed to obtain emails changes: {}", err);
                        self.write_bytes(err.into_status_response().into_bytes())
                            .await;
                        return;
                    }
                };

                // Synchronize messages
                let new_state = match self.synchronize_messages(mailbox.id.clone()).await {
                    Ok(new_state) => new_state,
                    Err(err) => {
                        self.write_bytes(err.into_bytes()).await;
                        return;
                    }
                };

                // Update UIDs
                let mut buf = Vec::with_capacity(64);
                let (new_message_count, deletions) =
                    mailbox.synchronize_uids(new_state.jmap_ids, new_state.imap_uids, true);
                if let Some(deletions) = deletions {
                    expunge::Response {
                        is_qresync,
                        ids: deletions
                            .into_iter()
                            .map(|id| if !is_qresync { id.seqnum } else { id.uid })
                            .collect(),
                    }
                    .serialize_to(&mut buf);
                }
                if let Some(new_message_count) = new_message_count {
                    Exists {
                        total_messages: new_message_count,
                    }
                    .serialize(&mut buf);
                }
                if !buf.is_empty() {
                    self.write_bytes(buf).await;
                }

                if response.total_changes() > 0 {
                    // Obtain ids of changed emails
                    let mut changed_ids = Vec::with_capacity(response.total_changes());
                    {
                        // Update state
                        let mut state = mailbox.state.lock();
                        state.last_state = response.take_new_state();
                        for (pos, jmap_id) in state.jmap_ids.iter().enumerate() {
                            if response.updated().contains(jmap_id)
                                || response.created().contains(jmap_id)
                            {
                                changed_ids.push(Sequence::Number {
                                    value: state.imap_uids[pos],
                                });
                            }
                        }
                    }

                    if !changed_ids.is_empty() {
                        self.fetch(
                            fetch::Arguments {
                                tag: String::new(),
                                sequence_set: Sequence::List { items: changed_ids },
                                attributes: vec![fetch::Attribute::Flags, fetch::Attribute::Uid],
                                changed_since: None,
                                include_vanished: false,
                            },
                            mailbox.clone(),
                            true,
                            is_qresync,
                            false,
                        )
                        .await;
                    }
                }
            }
        }
    }
}
