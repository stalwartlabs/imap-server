use crate::{
    core::{
        client::{Session, State},
        receiver::Request,
        Command, ResponseCode, StatusResponse,
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
                if let Some(mailbox) = data.get_mailbox_by_name(&arguments.mailbox_name) {
                    // Syncronize messages
                    let mailbox = Arc::new(mailbox);
                    match data.synchronize_messages(mailbox.clone()).await {
                        Ok(status) => {
                            let closed_previous = self.state.is_mailbox_selected();

                            // Obtain highest modseq
                            let highest_modseq = if self.is_condstore || arguments.condstore {
                                match data.synchronize_state(&mailbox.account_id).await {
                                    Ok(highest_modseq) => highest_modseq.into(),
                                    Err(mut response) => {
                                        response.tag = arguments.tag.into();
                                        return self.write_bytes(response.into_bytes()).await;
                                    }
                                }
                            } else {
                                None
                            };

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
                                if qresync.uid_validity == status.uid_validity {
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
                                        is_select,
                                        true,
                                        true,
                                    )
                                    .await;
                                }
                            }

                            // Update state
                            *data.saved_search.lock() = SavedSearch::None;
                            self.state = State::Selected {
                                data,
                                mailbox,
                                rw: is_select,
                                condstore: arguments.condstore,
                            };

                            self.write_bytes(
                                StatusResponse::completed(command)
                                    .with_tag(arguments.tag)
                                    .with_code(if is_select {
                                        ResponseCode::ReadWrite
                                    } else {
                                        ResponseCode::ReadOnly
                                    })
                                    .serialize(
                                        Response {
                                            mailbox: ListItem::new(arguments.mailbox_name),
                                            total_messages: status.total_messages,
                                            recent_messages: 0,
                                            unseen_seq: 0,
                                            uid_validity: status.uid_validity,
                                            uid_next: status.uid_next,
                                            closed_previous,
                                            is_rev2: self.version.is_rev2(),
                                            highest_modseq,
                                        }
                                        .serialize(),
                                    ),
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
