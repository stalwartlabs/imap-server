use jmap_client::email::Property;

use crate::{
    core::{client::Session, receiver::Request, Flag, IntoStatusResponse, StatusResponse},
    protocol::{
        fetch::{DataItem, FetchItem},
        store::{self, Operation},
        ImapResponse,
    },
};

impl Session {
    pub async fn handle_store(&mut self, request: Request, is_uid: bool) -> Result<(), ()> {
        match request.parse_store() {
            Ok(arguments) => {
                let (data, mailbox, _) = self.state.mailbox_data();
                let version = self.version;

                tokio::spawn(async move {
                    let mut request = data.client.build();
                    let set_request = request.set_email().account_id(&mailbox.account_id);
                    let keywords = arguments
                        .keywords
                        .iter()
                        .map(|k| k.to_jmap())
                        .collect::<Vec<_>>();
                    let update_ids = vec![(0u32, Some("".to_string()))];
                    for (_, jmap_id) in &update_ids {
                        if let Some(jmap_id) = jmap_id {
                            let update_item = set_request.update(jmap_id);
                            let is_set = match arguments.operation {
                                Operation::Set => {
                                    update_item
                                        .keywords(arguments.keywords.iter().map(|k| k.to_jmap()));
                                    continue;
                                }
                                Operation::Add => true,
                                Operation::Clear => false,
                            };
                            for keyword in &keywords {
                                update_item.keyword(keyword, is_set);
                            }
                        }
                    }

                    match request.send_set_email().await {
                        Ok(response) => match response.updated_ids() {
                            Some(updated_ids) if !arguments.is_silent => {
                                let mut request = data.client.build();
                                request
                                    .get_email()
                                    .account_id(&mailbox.account_id)
                                    .ids(updated_ids)
                                    .properties([Property::Id, Property::Keywords]);

                                match request.send_get_email().await {
                                    Ok(response) => {
                                        let mut items = Vec::with_capacity(response.list().len());
                                        for email in response.list() {
                                            if let Some((id, _)) =
                                                update_ids.iter().find(|(_, jmap_id)| {
                                                    jmap_id.as_deref() == email.id()
                                                })
                                            {
                                                items.push(FetchItem {
                                                    id: *id,
                                                    items: vec![DataItem::Flags {
                                                        flags: email
                                                            .keywords()
                                                            .iter()
                                                            .map(|k| {
                                                                Flag::parse_jmap(k.to_string())
                                                            })
                                                            .collect(),
                                                    }],
                                                });
                                            }
                                        }

                                        data.write_bytes(
                                            store::Response { items }
                                                .serialize(arguments.tag, version),
                                        )
                                        .await;
                                    }
                                    Err(err) => {
                                        data.write_bytes(
                                            err.into_status_response(arguments.tag.into())
                                                .into_bytes(),
                                        )
                                        .await;
                                    }
                                }
                            }
                            _ => {
                                data.write_bytes(
                                    StatusResponse::ok(
                                        arguments.tag.into(),
                                        None,
                                        "STORE completed",
                                    )
                                    .into_bytes(),
                                )
                                .await;
                            }
                        },
                        Err(err) => {
                            data.write_bytes(
                                err.into_status_response(arguments.tag.into()).into_bytes(),
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
