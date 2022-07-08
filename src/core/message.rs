use std::collections::{HashMap, HashSet};

use jmap_client::email::query::Filter;
use tracing::error;

use crate::core::ResponseCode;

use super::{client::SessionData, IntoStatusResponse, StatusResponse};

pub struct MailboxState {
    pub state_id: String,
    pub uid_next: u32,
    pub total_messages: usize,
}

const JMAP_TO_UID: u8 = 0;
const UID_TO_JMAP: u8 = 1;

impl SessionData {
    pub async fn synchronize_messages(
        &self,
        account_id: &str,
        mailbox_id: Option<&str>,
    ) -> Result<MailboxState, StatusResponse> {
        let mut valid_ids = HashSet::new();
        let mut state_id = String::new();
        let mut position = 0;
        let mut total_messages = 0;

        // Fetch all ids in the mailbox.
        for _ in 0..100 {
            let mut request = self.client.build().account_id(account_id);
            let query_request = request
                .query_email()
                .calculate_total(true)
                .position(position);
            if let Some(mailbox_id) = mailbox_id {
                query_request.filter(Filter::in_mailbox(mailbox_id));
            }

            let mut response = request
                .send_query_email()
                .await
                .map_err(|err| err.into_status_response(None))?;
            state_id = response.unwrap_query_state();
            total_messages = response.total().unwrap_or(0);
            let emails = response.unwrap_ids();

            let emails_len = emails.len();
            if emails_len > 0 {
                valid_ids.extend(emails.into_iter().map(|id| id.into_bytes()));
                if valid_ids.len() < total_messages {
                    position += emails_len as i32;
                    continue;
                }
            }
            break;
        }

        // Remove from cache messages no longer present in the mailbox.
        let prefix = serialize_key_prefix(account_id, mailbox_id, JMAP_TO_UID);
        let mut prefix_end = prefix.clone();
        prefix_end.push(u8::MAX);
        for kv_result in self.core.db.range(&prefix[..]..&prefix_end[..]) {
            let (key, value) = kv_result.map_err(|err| {
                error!("Failed to scan db: {}", err);
                StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
            })?;
            if key.starts_with(&prefix) {
                if key.len() > prefix.len() && !valid_ids.remove(&key[prefix.len()..]) {
                    for key in [
                        &key[..],
                        &serialize_key(account_id, mailbox_id, UID_TO_JMAP, &value)[..],
                    ] {
                        self.core.db.remove(key).map_err(|err| {
                            error!("Failed to delete key: {}", err);
                            StatusResponse::no(
                                None,
                                ResponseCode::ContactAdmin.into(),
                                "Database failure.",
                            )
                        })?;
                    }
                }
            } else {
                break;
            }
        }

        // Add to db any new ids.
        let uid_next_key = serialize_uid_next_key(account_id, mailbox_id);
        for jmap_id in valid_ids {
            self.insert_jmap_id(account_id, mailbox_id, &jmap_id, &uid_next_key)?;
        }

        Ok(MailboxState {
            state_id,
            uid_next: self.uid_next(account_id, mailbox_id).ok_or_else(|| {
                error!("Failed to generate UID.");
                StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
            })?,
            total_messages,
        })
    }

    pub fn jmap_to_uid(
        &self,
        account_id: &str,
        mailbox_id: Option<&str>,
        jmap_ids: &[&str],
    ) -> Result<Vec<u32>, StatusResponse> {
        let mut uids = Vec::with_capacity(jmap_ids.len());
        let mut uid_next_key = None;
        for jmap_id in jmap_ids {
            let jmap_id = jmap_id.as_bytes();
            let uid = if let Some(uid) = self
                .core
                .db
                .get(serialize_key(account_id, mailbox_id, JMAP_TO_UID, jmap_id))
                .map_err(|err| {
                    error!("Failed to get key: {}", err);
                    StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
                })? {
                uid
            } else {
                self.insert_jmap_id(
                    account_id,
                    mailbox_id,
                    jmap_id,
                    uid_next_key
                        .get_or_insert_with(|| serialize_uid_next_key(account_id, mailbox_id)),
                )?
            };
            uids.push(u32::from_be_bytes((&uid[..]).try_into().map_err(|_| {
                error!("Failed to convert bytes to u32.");
                StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
            })?));
        }

        Ok(uids)
    }

    pub fn uid_to_jmap(
        &self,
        account_id: &str,
        mailbox_id: Option<&str>,
        uids: &[u32],
    ) -> Result<Vec<Option<String>>, StatusResponse> {
        let mut jmap_ids = Vec::with_capacity(uids.len());
        for uid in uids {
            jmap_ids.push(
                if let Some(jmap_id) = self
                    .core
                    .db
                    .get(serialize_key(
                        account_id,
                        mailbox_id,
                        UID_TO_JMAP,
                        &uid.to_be_bytes()[..],
                    ))
                    .map_err(|err| {
                        error!("Failed to get key: {}", err);
                        StatusResponse::no(
                            None,
                            ResponseCode::ContactAdmin.into(),
                            "Database failure.",
                        )
                    })?
                {
                    String::from_utf8(jmap_id.to_vec())
                        .map_err(|_| {
                            error!("Failed to convert bytes to string.");
                            StatusResponse::no(
                                None,
                                ResponseCode::ContactAdmin.into(),
                                "Database failure.",
                            )
                        })?
                        .into()
                } else {
                    None
                },
            );
        }
        Ok(jmap_ids)
    }

    pub fn seqnum_to_jmap(
        &self,
        account_id: &str,
        mailbox_id: Option<&str>,
        seq_nums: &[u32],
    ) -> Result<Vec<Option<String>>, StatusResponse> {
        let prefix = serialize_key_prefix(account_id, mailbox_id, UID_TO_JMAP);
        let mut prefix_end = prefix.clone();
        prefix_end.push(u8::MAX);
        let mut seq_num = 0;
        let mut seq_nums_map = HashMap::with_capacity(seq_nums.len());

        for kv_result in self.core.db.range(&prefix[..]..&prefix_end[..]) {
            let (key, value) = kv_result.map_err(|err| {
                error!("Failed to scan db: {}", err);
                StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
            })?;
            if key.starts_with(&prefix) && key.len() > prefix.len() {
                seq_num += 1;

                if seq_nums.contains(&seq_num) {
                    seq_nums_map.insert(
                        seq_num,
                        String::from_utf8(value.to_vec()).map_err(|_| {
                            error!("Failed to convert bytes to string.");
                            StatusResponse::no(
                                None,
                                ResponseCode::ContactAdmin.into(),
                                "Database failure.",
                            )
                        })?,
                    );
                    if seq_nums_map.len() == seq_nums.len() {
                        break;
                    }
                }
            } else {
                break;
            }
        }
        Ok(seq_nums
            .iter()
            .map(|seq_num| seq_nums_map.remove(seq_num))
            .collect())
    }

    fn insert_jmap_id(
        &self,
        account_id: &str,
        mailbox_id: Option<&str>,
        jmap_id: &[u8],
        uid_next_key: &[u8],
    ) -> Result<sled::IVec, StatusResponse> {
        // Obtain next UID.
        let uid = self
            .core
            .db
            .update_and_fetch(&uid_next_key, increment_uid)
            .map_err(|err| {
                error!("Failed to increment UID: {}", err);
                StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
            })?
            .ok_or_else(|| {
                error!("Failed to generate UID.");
                StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
            })?;

        // Write keys
        for result in [
            self.core.db.insert(
                serialize_key(account_id, mailbox_id, JMAP_TO_UID, jmap_id),
                &uid,
            ),
            self.core.db.insert(
                serialize_key(account_id, mailbox_id, UID_TO_JMAP, &uid),
                jmap_id,
            ),
        ] {
            result.map_err(|err| {
                error!("Failed to insert key: {}", err);
                StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
            })?;
        }
        Ok(uid)
    }

    pub fn uid_next(&self, account_id: &str, mailbox_id: Option<&str>) -> Option<u32> {
        if let Some(bytes) = self
            .core
            .db
            .get(serialize_uid_next_key(account_id, mailbox_id))
            .ok()?
        {
            (u32::from_be_bytes((&bytes[..]).try_into().ok()?) + 1).into()
        } else {
            0.into()
        }
    }
}

fn serialize_key_prefix(account_id: &str, mailbox_id: Option<&str>, separator: u8) -> Vec<u8> {
    let mut buf = Vec::with_capacity(account_id.len() + mailbox_id.map_or(0, |m| m.len()) + 2);
    buf.extend_from_slice(account_id.as_bytes());
    buf.push(0);
    if let Some(mailbox_id) = mailbox_id {
        buf.extend_from_slice(mailbox_id.as_bytes());
    }
    buf.push(separator);
    buf
}

fn serialize_key(
    account_id: &str,
    mailbox_id: Option<&str>,
    separator: u8,
    value: &[u8],
) -> Vec<u8> {
    let mut buf =
        Vec::with_capacity(account_id.len() + mailbox_id.map_or(0, |m| m.len()) + value.len() + 2);
    buf.extend_from_slice(account_id.as_bytes());
    buf.push(0);
    if let Some(mailbox_id) = mailbox_id {
        buf.extend_from_slice(mailbox_id.as_bytes());
    }
    buf.push(separator);
    buf.extend_from_slice(value);
    buf
}

fn serialize_uid_next_key(account_id: &str, mailbox_id: Option<&str>) -> Vec<u8> {
    let mut buf = Vec::with_capacity(account_id.len() + mailbox_id.map_or(0, |m| m.len()) + 1);
    buf.extend_from_slice(account_id.as_bytes());
    buf.push(0);
    if let Some(mailbox_id) = mailbox_id {
        buf.extend_from_slice(mailbox_id.as_bytes());
    }
    buf
}

fn increment_uid(old: Option<&[u8]>) -> Option<Vec<u8>> {
    match old {
        Some(bytes) => u32::from_be_bytes(bytes.try_into().ok()?) + 1,
        None => 0,
    }
    .to_be_bytes()
    .to_vec()
    .into()
}
