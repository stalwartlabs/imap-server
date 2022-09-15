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

use tracing::debug;

use crate::{
    core::{
        client::{Session, SessionData},
        receiver::Request,
        IntoStatusResponse, ResponseCode, StatusResponse,
    },
    protocol::delete::Arguments,
};

impl Session {
    pub async fn handle_delete(&mut self, requests: Vec<Request>) -> Result<(), ()> {
        let mut arguments = Vec::with_capacity(requests.len());

        for request in requests {
            match request.parse_delete(self.version) {
                Ok(argument) => {
                    arguments.push(argument);
                }
                Err(response) => self.write_bytes(response.into_bytes()).await?,
            }
        }

        if !arguments.is_empty() {
            let data = self.state.session_data();
            tokio::spawn(async move {
                for argument in arguments {
                    data.write_bytes(data.delete_folder(argument).await.into_bytes())
                        .await;
                }
            });
        }
        Ok(())
    }
}

impl SessionData {
    pub async fn delete_folder(&self, arguments: Arguments) -> StatusResponse {
        // Refresh mailboxes
        if let Err(err) = self.synchronize_mailboxes(false, false).await {
            debug!("Failed to refresh mailboxes: {}", err);
            return err.into_status_response().with_tag(arguments.tag);
        }

        // Validate mailbox
        let mut delete_uid_cache = false;
        let (account_id, mailbox_id) = {
            let prefix = format!("{}/", arguments.mailbox_name);
            let mut mailbox_id = None;
            'outer: for account in self.mailboxes.lock().iter() {
                if account
                    .prefix
                    .as_ref()
                    .map_or(true, |p| arguments.mailbox_name.starts_with(p))
                {
                    for (mailbox_name, mailbox_id_) in account.mailbox_names.iter() {
                        if mailbox_name == &arguments.mailbox_name {
                            delete_uid_cache = account.prefix.is_none();
                            mailbox_id =
                                (account.account_id.to_string(), mailbox_id_.to_string()).into();
                            break 'outer;
                        } else if mailbox_name.starts_with(&prefix) {
                            return StatusResponse::no(
                                "Mailbox has children that need to be deleted first.",
                            )
                            .with_tag(arguments.tag)
                            .with_code(ResponseCode::HasChildren);
                        }
                    }
                }
            }
            if let Some(mailbox_id) = mailbox_id {
                mailbox_id
            } else {
                return StatusResponse::no("Mailbox does not exist.").with_tag(arguments.tag);
            }
        };

        // Delete mailbox
        if let Err(err) = self.client.mailbox_destroy(&mailbox_id, true).await {
            return err.into_status_response().with_tag(arguments.tag);
        }

        // Delete UID cache
        if delete_uid_cache {
            self.core
                .delete_mailbox(&account_id, &mailbox_id)
                .await
                .ok();
        }

        // Update mailbox cache
        for account in self.mailboxes.lock().iter_mut() {
            if account.account_id == account_id {
                account.mailbox_names.remove(&arguments.mailbox_name);
                account.mailbox_data.remove(&mailbox_id);
                break;
            }
        }

        StatusResponse::ok("Mailbox deleted.").with_tag(arguments.tag)
    }
}
