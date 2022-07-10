use tracing::debug;

use crate::core::{
    client::{Session, SessionData},
    receiver::Request,
    IntoStatusResponse, ResponseCode, StatusResponse,
};

impl Session {
    pub async fn handle_subscribe(
        &mut self,
        request: Request,
        is_subscribe: bool,
    ) -> Result<(), ()> {
        match request.parse_subscribe(self.version) {
            Ok(arguments) => {
                let data = self.state.session_data();
                tokio::spawn(async move {
                    data.write_bytes(
                        data.subscribe_folder(arguments.tag, arguments.mailbox_name, is_subscribe)
                            .await
                            .into_bytes(),
                    )
                    .await;
                });
                Ok(())
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}

impl SessionData {
    pub async fn subscribe_folder(
        &self,
        tag: String,
        mailbox_name: String,
        subscribe: bool,
    ) -> StatusResponse {
        // Refresh mailboxes
        if let Err(err) = self.synchronize_mailboxes().await {
            debug!("Failed to refresh mailboxes: {}", err);
            return err.into_status_response(tag.into());
        }

        // Validate mailbox
        let (account_id, mailbox_id) = match self.get_mailbox_by_name(&mailbox_name) {
            Some(mailbox) => {
                if let Some(mailbox_id) = mailbox.mailbox_id {
                    (mailbox.account_id, mailbox_id)
                } else {
                    return StatusResponse::no(
                        tag.into(),
                        ResponseCode::Cannot.into(),
                        "Subscribing to this mailbox is not supported.",
                    );
                }
            }
            None => {
                return StatusResponse::no(
                    tag.into(),
                    ResponseCode::NonExistent.into(),
                    "Mailbox does not exist.",
                );
            }
        };

        // [Un]subscribe mailbox
        if let Err(err) = self.client.mailbox_subscribe(&mailbox_id, subscribe).await {
            return err.into_status_response(tag.into());
        }

        // Update mailbox cache
        for account in self.mailboxes.lock().iter_mut() {
            if account.account_id == account_id {
                if let Some(mailbox) = account.mailbox_data.get_mut(&mailbox_id) {
                    mailbox.is_subscribed = subscribe;
                }
                break;
            }
        }

        StatusResponse::ok(
            tag.into(),
            None,
            if subscribe {
                "Mailbox subscribed."
            } else {
                "Mailbox unsubscribed."
            },
        )
    }
}
