use tracing::debug;

use crate::core::{
    client::{Session, SessionData},
    receiver::Request,
    IntoStatusResponse, ResponseCode, StatusResponse,
};

impl Session {
    pub async fn handle_subscribe(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_subscribe(self.version) {
            Ok(arguments) => {
                let data = self.state.session_data();
                tokio::spawn(async move {
                    data.write_bytes(
                        data.subscribe_folder(arguments.tag, arguments.mailbox_name, true)
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

    pub async fn handle_unsubscribe(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_unsubscribe(self.version) {
            Ok(arguments) => {
                let data = self.state.session_data();
                tokio::spawn(async move {
                    data.write_bytes(
                        data.subscribe_folder(arguments.tag, arguments.mailbox_name, false)
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
        let (account_id, mailbox_id) =
            if let Some(mailbox_id) = self.get_mailbox_by_name(&mailbox_name) {
                mailbox_id
            } else {
                let is_all = self.is_all_mailbox(&mailbox_name);
                return StatusResponse::no(
                    tag.into(),
                    if !is_all {
                        ResponseCode::NonExistent
                    } else {
                        ResponseCode::Cannot
                    }
                    .into(),
                    if !is_all {
                        "Mailbox does not exist."
                    } else {
                        "Subscribing to this mailbox is not supported."
                    },
                );
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
