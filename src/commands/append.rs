use std::sync::Arc;

use tracing::debug;

use crate::core::{
    client::Session, receiver::Request, Command, IntoStatusResponse, ResponseCode, StatusResponse,
};

impl Session {
    pub async fn handle_append(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_append() {
            Ok(arguments) => {
                let data = self.state.session_data();

                // Refresh mailboxes
                if let Err(err) = data.synchronize_mailboxes(false).await {
                    debug!("Failed to refresh mailboxes: {}", err);
                    return self
                        .write_bytes(
                            err.into_status_response()
                                .with_tag(arguments.tag)
                                .into_bytes(),
                        )
                        .await;
                }

                let mailbox =
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
                                StatusResponse::no("Mailbox does not exist.")
                                    .with_tag(arguments.tag)
                                    .with_code(ResponseCode::TryCreate)
                                    .into_bytes(),
                            )
                            .await;
                    };

                tokio::spawn(async move {
                    match data
                        .client
                        .email_import_account(
                            &mailbox.account_id,
                            arguments.message,
                            [mailbox.mailbox_id.as_ref().unwrap()],
                            arguments.flags.iter().map(|f| f.to_jmap()).into(),
                            arguments.received_at,
                        )
                        .await
                    {
                        Ok(mut email) => {
                            let jmap_id = email.take_id();
                            let mut response =
                                StatusResponse::completed(Command::Append).with_tag(arguments.tag);
                            if !jmap_id.is_empty() {
                                if let Ok(ids) = data
                                    .core
                                    .jmap_to_imap(mailbox.clone(), vec![jmap_id], true, true)
                                    .await
                                {
                                    if let Ok((uid_validity, _)) = data.core.uids(mailbox).await {
                                        response = response.with_code(ResponseCode::AppendUid {
                                            uid_validity,
                                            uids: ids.uids,
                                        });
                                    }
                                }
                            }
                            data.write_bytes(response.into_bytes()).await;
                        }
                        Err(response) => {
                            data.write_bytes(
                                response
                                    .into_status_response()
                                    .with_tag(arguments.tag)
                                    .into_bytes(),
                            )
                            .await;
                        }
                    }
                });
                Ok(())
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}
