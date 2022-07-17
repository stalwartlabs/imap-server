use std::sync::Arc;

use jmap_client::{core::query, email::query::Filter};

use crate::{
    core::{
        client::{Session, SessionData},
        message::MailboxData,
        receiver::Request,
        Command, Flag, IntoStatusResponse, ResponseCode, StatusResponse,
    },
    protocol::{expunge::Response, ImapResponse},
};

impl Session {
    pub async fn handle_expunge(&mut self, request: Request, is_uid: bool) -> Result<(), ()> {
        let (data, mailbox, _) = self.state.mailbox_data();
        match data.expunge(mailbox.clone()).await {
            Ok(Some(jmap_ids)) if !jmap_ids.is_empty() => {
                match data
                    .core
                    .jmap_to_imap(mailbox, jmap_ids, false, is_uid)
                    .await
                {
                    Ok(ids) => {
                        self.write_bytes(
                            Response {
                                is_uid,
                                ids: if is_uid {
                                    ids.uids
                                } else {
                                    ids.seqnums.unwrap()
                                },
                            }
                            .serialize(request.tag),
                        )
                        .await
                    }
                    Err(_) => {
                        self.write_bytes(
                            StatusResponse::database_failure(request.tag.into()).into_bytes(),
                        )
                        .await
                    }
                }
            }
            Ok(_) => {
                self.write_bytes(
                    StatusResponse::completed(Command::Expunge(is_uid), request.tag).into_bytes(),
                )
                .await
            }
            Err(mut response) => {
                response.tag = request.tag.into();
                self.write_bytes(response.into_bytes()).await
            }
        }
    }
}

impl SessionData {
    pub async fn expunge(
        &self,
        mailbox: Arc<MailboxData>,
    ) -> crate::core::Result<Option<Vec<String>>> {
        let mut request = self.client.build();
        let result_ref = request
            .query_email()
            .account_id(&mailbox.account_id)
            .filter(query::Filter::and(
                if let Some(mailbox_id) = &mailbox.mailbox_id {
                    vec![
                        Filter::in_mailbox(mailbox_id),
                        Filter::has_keyword(Flag::Deleted.to_jmap()),
                    ]
                } else {
                    vec![Filter::has_keyword(Flag::Deleted.to_jmap())]
                },
            ))
            .result_reference();
        request.set_email().destroy_ref(result_ref);
        let mut response = request
            .send()
            .await
            .map_err(|err| err.into_status_response(None))?
            .unwrap_method_responses();
        if response.len() != 2 {
            return Err(StatusResponse::no(
                None,
                ResponseCode::ContactAdmin.into(),
                "Invalid JMAP server response",
            ));
        }

        Ok(response
            .pop()
            .unwrap()
            .unwrap_set_email()
            .map_err(|err| err.into_status_response(None))?
            .take_destroyed_ids())
    }
}
