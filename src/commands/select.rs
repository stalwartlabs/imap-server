use tracing::debug;

use crate::{
    core::{
        client::{SelectedMailbox, Session, State},
        receiver::Request,
        Command, IntoStatusResponse, ResponseCode, StatusResponse,
    },
    protocol::{fetch, list::ListItem, select::Response, ImapResponse},
};
use std::sync::Arc;

use super::search::SavedSearch;

impl Session {
    pub async fn handle_select(&mut self, request: Request) -> Result<(), ()> {
        let is_select = request.command == Command::Select;
        let command = request.command;
        match request.parse_select(self.version) {
            Ok(arguments) => {
                let data = self.state.session_data();

                // Refresh mailboxes
                if let Err(err) = data.synchronize_mailboxes(false, false).await {
                    debug!("Failed to synchronize mailboxes: {}", err);
                    return self
                        .write_bytes(
                            err.into_status_response()
                                .with_tag(arguments.tag)
                                .into_bytes(),
                        )
                        .await;
                }

                if let Some(mailbox) = data.get_mailbox_by_name(&arguments.mailbox_name) {
                    // Syncronize messages
                    let mailbox = Arc::new(mailbox);
                    match data.synchronize_messages(mailbox.clone()).await {
                        Ok(mut state) => {
                            let closed_previous = self.state.is_mailbox_selected();
                            let is_condstore = self.is_condstore || arguments.condstore;

                            // Obtain JMAP state
                            state.last_state = match data.get_jmap_state(&mailbox.account_id).await
                            {
                                Ok(jmap_state) => jmap_state,
                                Err(mut response) => {
                                    response.tag = arguments.tag.into();
                                    return self.write_bytes(response.into_bytes()).await;
                                }
                            };

                            // Obtain highest modseq
                            let highest_modseq = if is_condstore {
                                match data
                                    .core
                                    .state_to_modseq(&mailbox.account_id, state.last_state.clone())
                                    .await
                                {
                                    Ok(highest_modseq) => highest_modseq.into(),
                                    Err(_) => {
                                        return self
                                            .write_bytes(
                                                StatusResponse::database_failure()
                                                    .with_tag(arguments.tag)
                                                    .into_bytes(),
                                            )
                                            .await;
                                    }
                                }
                            } else {
                                None
                            };

                            // Build new state
                            let uid_validity = state.uid_validity;
                            let uid_next = state.uid_next;
                            let total_messages = state.imap_uids.len();
                            let mailbox = Arc::new(SelectedMailbox {
                                id: mailbox,
                                state: parking_lot::Mutex::new(state),
                                saved_search: parking_lot::Mutex::new(SavedSearch::None),
                                is_select,
                                is_condstore,
                            });

                            // Validate QRESYNC arguments
                            if let Some(qresync) = arguments.qresync {
                                if !self.is_qresync {
                                    return self
                                        .write_bytes(
                                            StatusResponse::no("QRESYNC is not enabled.")
                                                .with_tag(arguments.tag)
                                                .into_bytes(),
                                        )
                                        .await;
                                }
                                if qresync.uid_validity == uid_validity {
                                    // Send flags for changed messages
                                    data.fetch(
                                        fetch::Arguments {
                                            tag: String::new(),
                                            sequence_set: qresync
                                                .known_uids
                                                .unwrap_or_else(|| qresync.seq_match.unwrap().1),
                                            attributes: vec![fetch::Attribute::Flags],
                                            changed_since: qresync.modseq.into(),
                                            include_vanished: true,
                                        },
                                        mailbox.clone(),
                                        true,
                                        true,
                                    )
                                    .await;
                                }
                            }

                            // Build response
                            let response = Response {
                                mailbox: ListItem::new(arguments.mailbox_name),
                                total_messages,
                                recent_messages: 0,
                                unseen_seq: 0,
                                uid_validity,
                                uid_next,
                                closed_previous,
                                is_rev2: self.version.is_rev2(),
                                highest_modseq,
                                mailbox_id: if let Some(mailbox_id) = &mailbox.id.mailbox_id {
                                    format!("{}-{}", mailbox.id.account_id, mailbox_id)
                                } else {
                                    mailbox.id.account_id.clone()
                                },
                            };

                            // Update state
                            self.state = State::Selected { data, mailbox };

                            self.write_bytes(
                                StatusResponse::completed(command)
                                    .with_tag(arguments.tag)
                                    .with_code(if is_select {
                                        ResponseCode::ReadWrite
                                    } else {
                                        ResponseCode::ReadOnly
                                    })
                                    .serialize(response.serialize()),
                            )
                            .await
                        }
                        Err(mut response) => {
                            response.tag = arguments.tag.into();
                            self.write_bytes(response.into_bytes()).await
                        }
                    }
                } else {
                    self.write_bytes(
                        StatusResponse::no("Mailbox does not exist.")
                            .with_tag(arguments.tag)
                            .with_code(ResponseCode::NonExistent)
                            .into_bytes(),
                    )
                    .await
                }
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}
