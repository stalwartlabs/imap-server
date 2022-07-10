use std::sync::Arc;

use jmap_client::{core::query, email::query::Filter};

use crate::{
    core::{
        client::{Session, SessionData},
        message::MailboxData,
        receiver::Request,
        IntoStatusResponse, ResponseCode, StatusResponse,
    },
    protocol::{expunge, ImapResponse},
};

impl Session {
    pub async fn handle_expunge(&mut self, request: Request, is_uid: bool) -> Result<(), ()> {
        let (data, mailbox, _) = self.state.mailbox_data();
        match data.expunge(mailbox.clone()).await {
            Ok(Some(jmap_ids)) if !jmap_ids.is_empty() => {
                match if is_uid {
                    data.core.jmap_to_uid(mailbox, jmap_ids, false).await
                } else {
                    data.core.jmap_to_seqnum(mailbox, jmap_ids, false).await
                } {
                    Ok(ids) => {
                        self.write_bytes(
                            expunge::Response { ids }.serialize(request.tag, self.version),
                        )
                        .await
                    }
                    Err(mut response) => {
                        response.tag = request.tag.into();
                        self.write_bytes(response.into_bytes()).await
                    }
                }
            }
            Ok(_) => {
                self.write_bytes(
                    StatusResponse::ok(request.tag.into(), None, "EXPUNGE completed").into_bytes(),
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
                        Filter::has_keyword("$deleted"),
                    ]
                } else {
                    vec![Filter::has_keyword("$deleted")]
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
            .unwrap_destroyed_ids())
    }
}
