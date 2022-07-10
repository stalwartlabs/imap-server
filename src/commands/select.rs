use crate::{
    core::{
        client::{Session, State},
        receiver::Request,
        Command, ResponseCode, StatusResponse,
    },
    protocol::{
        list::ListItem,
        select::{self},
        ImapResponse,
    },
};
use std::sync::Arc;

impl Session {
    pub async fn handle_select(&mut self, request: Request) -> Result<(), ()> {
        let is_select = request.command == Command::Select;
        match request.parse_select(self.version) {
            Ok(arguments) => {
                let data = self.state.session_data();
                if let Some(mailbox) = data.get_mailbox_by_name(&arguments.mailbox_name) {
                    // Syncronize messages
                    let mailbox = Arc::new(mailbox);
                    match data.synchronize_messages(mailbox.clone()).await {
                        Ok(status) => {
                            let closed_previous = self.state.is_mailbox_selected();

                            // Update state
                            self.state = State::Selected {
                                data,
                                mailbox,
                                rw: is_select,
                            };

                            self.write_bytes(
                                select::Response {
                                    mailbox: ListItem::new(arguments.mailbox_name),
                                    total_messages: status.total_messages,
                                    recent_messages: 0,
                                    unseen_seq: 0,
                                    uid_validity: status.uid_validity,
                                    uid_next: status.uid_next,
                                    is_read_only: !is_select,
                                    is_examine: !is_select,
                                    closed_previous,
                                }
                                .serialize(arguments.tag, self.version),
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
                        StatusResponse::no(
                            arguments.tag.into(),
                            ResponseCode::NonExistent.into(),
                            "Mailbox does not exist.",
                        )
                        .into_bytes(),
                    )
                    .await
                }
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}
