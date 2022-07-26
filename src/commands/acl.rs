use std::sync::Arc;

use jmap_client::mailbox::Property;
use tracing::debug;

use crate::{
    core::{
        client::{Session, SessionData},
        message::MailboxId,
        receiver::Request,
        Command, IntoStatusResponse, StatusResponse,
    },
    parser::PushUnique,
    protocol::acl::{
        Arguments, AsImapRights, GetAclResponse, ListRightsResponse, ModRightsOp, MyRightsResponse,
        Rights,
    },
};

impl Session {
    pub async fn handle_get_acl(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_acl() {
            Ok(arguments) => {
                let data = self.state.session_data();
                let is_rev2 = self.version.is_rev2();

                tokio::spawn(async move {
                    let mailbox = match data.get_acl_mailbox(&arguments).await {
                        Ok(mailbox) => mailbox,
                        Err(err) => {
                            data.write_bytes(
                                StatusResponse::no(err).with_tag(arguments.tag).into_bytes(),
                            )
                            .await;
                            return;
                        }
                    };
                    let mut request = data.client.build();
                    request
                        .get_mailbox()
                        .account_id(&mailbox.account_id)
                        .ids([mailbox.mailbox_id.as_ref().unwrap()])
                        .properties([Property::ACL]);
                    match request.send_get_mailbox().await {
                        Ok(mut response) => {
                            if let Some(mut mailbox) = response.take_list().pop() {
                                let mut permissions = Vec::new();

                                if let Some(acl) = mailbox.take_acl() {
                                    for (identifier, acls) in acl {
                                        let mut rights = Vec::with_capacity(acls.len());
                                        for acl in acls {
                                            let (right, other_right) = Rights::from_acl(acl);
                                            rights.push_unique(right);
                                            if let Some(other_right) = other_right {
                                                rights.push_unique(other_right);
                                            }
                                        }
                                        permissions.push((identifier, rights))
                                    }
                                }

                                data.write_bytes(
                                    StatusResponse::completed(Command::GetAcl)
                                        .with_tag(arguments.tag)
                                        .serialize(
                                            GetAclResponse {
                                                mailbox_name: arguments.mailbox_name,
                                                permissions,
                                            }
                                            .into_bytes(is_rev2),
                                        ),
                                )
                                .await;
                            } else {
                                data.write_bytes(
                                    StatusResponse::no("Mailbox not found")
                                        .with_tag(arguments.tag)
                                        .into_bytes(),
                                )
                                .await;
                            }
                        }
                        Err(err) => {
                            debug!("Failed to get ACL: {:?}", err);
                            data.write_bytes(
                                err.into_status_response()
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

    pub async fn handle_my_rights(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_acl() {
            Ok(arguments) => {
                let data = self.state.session_data();
                let is_rev2 = self.version.is_rev2();

                tokio::spawn(async move {
                    let mailbox = match data.get_acl_mailbox(&arguments).await {
                        Ok(mailbox) => mailbox,
                        Err(err) => {
                            data.write_bytes(
                                StatusResponse::no(err).with_tag(arguments.tag).into_bytes(),
                            )
                            .await;
                            return;
                        }
                    };
                    let mut request = data.client.build();
                    request
                        .get_mailbox()
                        .account_id(&mailbox.account_id)
                        .ids([mailbox.mailbox_id.as_ref().unwrap()])
                        .properties([Property::MyRights]);
                    match request.send_get_mailbox().await {
                        Ok(mut response) => {
                            if let Some(mailbox) = response.take_list().pop() {
                                data.write_bytes(
                                    StatusResponse::completed(Command::MyRights)
                                        .with_tag(arguments.tag)
                                        .serialize(
                                            MyRightsResponse {
                                                mailbox_name: arguments.mailbox_name,
                                                rights: if let Some(mailbox_rights) =
                                                    mailbox.my_rights()
                                                {
                                                    mailbox_rights.as_imap_rights()
                                                } else {
                                                    Vec::new()
                                                },
                                            }
                                            .into_bytes(is_rev2),
                                        ),
                                )
                                .await;
                            } else {
                                data.write_bytes(
                                    StatusResponse::no("Mailbox not found")
                                        .with_tag(arguments.tag)
                                        .into_bytes(),
                                )
                                .await;
                            }
                        }
                        Err(err) => {
                            debug!("Failed to get ACL: {:?}", err);
                            data.write_bytes(
                                err.into_status_response()
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

    pub async fn handle_set_acl(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_acl() {
            Ok(arguments) => {
                let data = self.state.session_data();

                tokio::spawn(async move {
                    let mailbox = match data.get_acl_mailbox(&arguments).await {
                        Ok(mailbox) => mailbox,
                        Err(err) => {
                            data.write_bytes(
                                StatusResponse::no(err).with_tag(arguments.tag).into_bytes(),
                            )
                            .await;
                            return;
                        }
                    };
                    let mailbox_id = mailbox.mailbox_id.as_ref().unwrap();
                    let mod_rights = arguments.mod_rights.unwrap();
                    let mut request = data.client.build();
                    let set_mailbox = request
                        .set_mailbox()
                        .account_id(&mailbox.account_id)
                        .update(mailbox_id);

                    match mod_rights.op {
                        ModRightsOp::Add => {
                            for right in mod_rights.rights {
                                set_mailbox.acl_set(
                                    arguments.identifier.as_ref().unwrap(),
                                    right.into_acl(),
                                    true,
                                );
                            }
                        }
                        ModRightsOp::Remove => {
                            for right in mod_rights.rights {
                                set_mailbox.acl_set(
                                    arguments.identifier.as_ref().unwrap(),
                                    right.into_acl(),
                                    false,
                                );
                            }
                        }
                        ModRightsOp::Replace => {
                            set_mailbox.acl(
                                &arguments.identifier.unwrap(),
                                mod_rights
                                    .rights
                                    .into_iter()
                                    .map(|r| r.into_acl())
                                    .collect::<Vec<_>>(),
                            );
                        }
                    }

                    match request.send_set_mailbox().await {
                        Ok(mut response) => match response.updated(mailbox_id) {
                            Ok(_) => {
                                data.write_bytes(
                                    StatusResponse::completed(Command::SetAcl)
                                        .with_tag(arguments.tag)
                                        .into_bytes(),
                                )
                                .await;
                            }
                            Err(err) => {
                                debug!("Failed to set ACL: {:?}", err);
                                data.write_bytes(
                                    err.into_status_response()
                                        .with_tag(arguments.tag)
                                        .into_bytes(),
                                )
                                .await;
                            }
                        },
                        Err(err) => {
                            debug!("Failed to set ACL: {:?}", err);
                            data.write_bytes(
                                err.into_status_response()
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

    pub async fn handle_delete_acl(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_acl() {
            Ok(arguments) => {
                let data = self.state.session_data();

                tokio::spawn(async move {
                    let mailbox = match data.get_acl_mailbox(&arguments).await {
                        Ok(mailbox) => mailbox,
                        Err(err) => {
                            data.write_bytes(
                                StatusResponse::no(err).with_tag(arguments.tag).into_bytes(),
                            )
                            .await;
                            return;
                        }
                    };
                    let mailbox_id = mailbox.mailbox_id.as_ref().unwrap();
                    let mut request = data.client.build();
                    request
                        .set_mailbox()
                        .account_id(&mailbox.account_id)
                        .update(mailbox_id)
                        .acl(arguments.identifier.as_ref().unwrap(), Vec::new());

                    match request.send_set_mailbox().await {
                        Ok(mut response) => match response.updated(mailbox_id) {
                            Ok(_) => {
                                data.write_bytes(
                                    StatusResponse::completed(Command::DeleteAcl)
                                        .with_tag(arguments.tag)
                                        .into_bytes(),
                                )
                                .await;
                            }
                            Err(err) => {
                                debug!("Failed to delete ACL: {:?}", err);
                                data.write_bytes(
                                    err.into_status_response()
                                        .with_tag(arguments.tag)
                                        .into_bytes(),
                                )
                                .await;
                            }
                        },
                        Err(err) => {
                            debug!("Failed to delete ACL: {:?}", err);
                            data.write_bytes(
                                err.into_status_response()
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

    pub async fn handle_list_rights(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_acl() {
            Ok(arguments) => {
                self.write_bytes(
                    StatusResponse::completed(Command::ListRights)
                        .with_tag(arguments.tag)
                        .serialize(
                            ListRightsResponse {
                                mailbox_name: arguments.mailbox_name,
                                identifier: arguments.identifier.unwrap(),
                                permissions: vec![
                                    vec![Rights::Read],
                                    vec![Rights::Lookup],
                                    vec![Rights::Write, Rights::Seen],
                                    vec![Rights::Insert],
                                    vec![Rights::Expunge, Rights::DeleteMessages],
                                    vec![Rights::CreateMailbox],
                                    vec![Rights::DeleteMailbox],
                                    vec![Rights::Post],
                                    vec![Rights::Administer],
                                ],
                            }
                            .into_bytes(self.version.is_rev2()),
                        ),
                )
                .await
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}

impl SessionData {
    pub async fn get_acl_mailbox(
        &self,
        arguments: &Arguments,
    ) -> Result<Arc<MailboxId>, &'static str> {
        if let Some(mailbox) = self.get_mailbox_by_name(&arguments.mailbox_name) {
            if mailbox.mailbox_id.is_some() {
                Ok(Arc::new(mailbox))
            } else {
                Err("ACL operations are not permitted on this mailbox.")
            }
        } else {
            Err("Mailbox does not exist.")
        }
    }
}
