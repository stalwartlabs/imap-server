use std::sync::Arc;

use crate::core::{
    client::Session, receiver::Request, IntoStatusResponse, ResponseCode, StatusResponse,
};

impl Session {
    pub async fn handle_append(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_append() {
            Ok(arguments) => {
                let data = self.state.session_data();
                let version = self.version;
                let mailbox =
                    if let Some(mailbox) = data.get_mailbox_by_name(&arguments.mailbox_name) {
                        if mailbox.mailbox_id.is_some() {
                            Arc::new(mailbox)
                        } else {
                            return self
                                .write_bytes(
                                    StatusResponse::no(
                                        arguments.tag.into(),
                                        ResponseCode::NoPerm.into(),
                                        "Appending messages to this mailbox is not allowed.",
                                    )
                                    .into_bytes(),
                                )
                                .await;
                        }
                    } else {
                        return self
                            .write_bytes(
                                StatusResponse::no(
                                    arguments.tag.into(),
                                    ResponseCode::NonExistent.into(),
                                    "Mailbox does not exist.",
                                )
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
                        Ok(email) => {
                            let jmap_id = email.unwrap_id();
                            let mut response_code = None;
                            if !jmap_id.is_empty() {
                                if let Ok(uids) = data
                                    .core
                                    .jmap_to_uid(mailbox.clone(), vec![jmap_id], true)
                                    .await
                                {
                                    if version.is_rev2() {
                                        if let Ok((uid_validity, _)) = data.core.uids(mailbox).await
                                        {
                                            response_code =
                                                ResponseCode::AppendUid { uid_validity, uids }
                                                    .into();
                                        }
                                    }
                                }
                            }
                            data.write_bytes(
                                StatusResponse::ok(
                                    arguments.tag.into(),
                                    response_code,
                                    "APPEND completed",
                                )
                                .into_bytes(),
                            )
                            .await;
                        }
                        Err(response) => {
                            data.write_bytes(
                                response
                                    .into_status_response(arguments.tag.into())
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
