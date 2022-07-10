use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::SystemTime,
};

use jmap_client::email::query::Filter;
use tokio::sync::oneshot;
use tracing::{debug, error};

use crate::core::ResponseCode;

use super::{client::SessionData, mailbox::Account, Core, IntoStatusResponse, StatusResponse};

pub struct MailboxData {
    pub account_id: String,
    pub mailbox_id: Option<String>,
}

pub struct MailboxStatus {
    pub state_id: String,
    pub uid_next: u32,
    pub uid_validity: u32,
    pub total_messages: usize,
}

const JMAP_TO_UID: u8 = 0;
const UID_TO_JMAP: u8 = 1;
const UID_NEXT: u8 = 2;
const UID_VALIDITY: u8 = 3;

impl SessionData {
    pub async fn synchronize_messages(
        &self,
        mailbox: Arc<MailboxData>,
    ) -> Result<MailboxStatus, StatusResponse> {
        let mut valid_ids = Vec::new();
        let mut state_id = String::new();
        let mut position = 0;
        let mut total_messages = 0;

        // Fetch all ids in the mailbox.
        for _ in 0..100 {
            let mut request = self.client.build().account_id(&mailbox.account_id);
            let query_request = request
                .query_email()
                .calculate_total(true)
                .position(position);
            if let Some(mailbox_id) = &mailbox.mailbox_id {
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
                valid_ids.extend(emails);
                if valid_ids.len() < total_messages {
                    position += emails_len as i32;
                    continue;
                }
            }
            break;
        }

        // Update mailbox
        let (uid_validity, uid_next) = self.core.update_uids(mailbox, valid_ids).await?;

        Ok(MailboxStatus {
            state_id,
            uid_next,
            uid_validity,
            total_messages,
        })
    }
}

impl Core {
    pub async fn update_uids(
        &self,
        mailbox: Arc<MailboxData>,
        jmap_ids: Vec<String>,
    ) -> Result<(u32, u32), StatusResponse> {
        let db = self.db.clone();
        self.spawn_worker(move || {
            // Obtain/generate UIDVALIDITY
            let uid_validity_key = serialize_uid_validity_key(&mailbox);
            let uid_validity = if let Some(uid_bytes) =
                db.get(&uid_validity_key).map_err(|err| {
                    error!("Failed to read key: {}", err);
                    StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
                })? {
                u32::from_be_bytes((&uid_bytes[..]).try_into().map_err(|err| {
                    error!("Failed to decode UID validity: {}", err);
                    StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
                })?)
            } else {
                // Number of hours since January 1st, 2000
                let uid_validity = (SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
                    .saturating_sub(946684800)
                    / 3600) as u32;
                db.insert(uid_validity_key, &uid_validity.to_be_bytes()[..])
                    .map_err(|err| {
                        error!("Failed to insert key: {}", err);
                        StatusResponse::no(
                            None,
                            ResponseCode::ContactAdmin.into(),
                            "Database failure.",
                        )
                    })?;
                uid_validity
            };

            // Remove from cache messages no longer present in the mailbox.
            let mut jmap_ids_map = jmap_ids
                .iter()
                .map(|id| id.as_bytes())
                .collect::<HashSet<_>>();

            let prefix = serialize_key_prefix(&mailbox, JMAP_TO_UID);
            let mut batch = sled::Batch::default();
            let mut has_deletions = false;

            for kv_result in db.scan_prefix(&prefix) {
                let (key, value) = kv_result.map_err(|err| {
                    error!("Failed to scan db: {}", err);
                    StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
                })?;
                if key.len() > prefix.len() && !jmap_ids_map.remove(&key[prefix.len()..]) {
                    for key in [&key[..], &serialize_key(&mailbox, UID_TO_JMAP, &value)[..]] {
                        batch.remove(key);
                        has_deletions = true;
                    }
                }
            }

            if has_deletions {
                db.apply_batch(batch).map_err(|err| {
                    error!("Failed to delete batch: {}", err);
                    StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
                })?;
            }

            // Add to the db any new ids.
            if !jmap_ids_map.is_empty() {
                #[cfg(test)]
                let jmap_ids_map = jmap_ids_map
                    .into_iter()
                    .collect::<std::collections::BTreeSet<_>>();

                let uid_next_key = serialize_uid_next_key(&mailbox);
                for jmap_id in jmap_ids_map {
                    db.insert_jmap_id(&mailbox, jmap_id, &uid_next_key)?;
                }
            }

            Ok((
                uid_validity,
                db.uid_next(&mailbox).ok_or_else(|| {
                    error!("Failed to generate UID.");
                    StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
                })?,
            ))
        })
        .await
    }

    pub async fn jmap_to_uid(
        &self,
        mailbox: Arc<MailboxData>,
        jmap_ids: Vec<String>,
        add_missing: bool,
    ) -> Result<Vec<u32>, StatusResponse> {
        let db = self.db.clone();
        self.spawn_worker(move || {
            let mut uids = Vec::with_capacity(jmap_ids.len());
            let mut uid_next_key = None;
            for jmap_id in jmap_ids {
                let jmap_id = jmap_id.as_bytes();
                let uid = if let Some(uid) = db
                    .get(serialize_key(&mailbox, JMAP_TO_UID, jmap_id))
                    .map_err(|err| {
                    error!("Failed to get key: {}", err);
                    StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
                })? {
                    uid
                } else if add_missing {
                    db.insert_jmap_id(
                        &mailbox,
                        jmap_id,
                        uid_next_key.get_or_insert_with(|| serialize_uid_next_key(&mailbox)),
                    )?
                } else {
                    continue;
                };
                uids.push(u32::from_be_bytes((&uid[..]).try_into().map_err(|_| {
                    error!("Failed to convert bytes to u32.");
                    StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
                })?));
            }

            Ok(uids)
        })
        .await
    }

    pub async fn jmap_to_seqnum(
        &self,
        mailbox: Arc<MailboxData>,
        jmap_ids: Vec<String>,
        add_missing: bool,
    ) -> Result<Vec<u32>, StatusResponse> {
        if jmap_ids.is_empty() {
            return Ok(Vec::new());
        }

        let db = self.db.clone();
        self.spawn_worker(move || {
            let prefix = serialize_key_prefix(&mailbox, UID_TO_JMAP);
            let mut jmap_ids_map = jmap_ids.iter().collect::<HashSet<_>>();
            let mut seq_num = 0;
            let mut seq_nums_map = HashMap::with_capacity(jmap_ids.len());

            for kv_result in db.scan_prefix(&prefix) {
                let (key, value) = kv_result.map_err(|err| {
                    error!("Failed to scan db: {}", err);
                    StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
                })?;
                if key.len() > prefix.len() {
                    seq_num += 1;

                    let value = String::from_utf8(value.to_vec()).map_err(|_| {
                        error!("Failed to convert bytes to string.");
                        StatusResponse::no(
                            None,
                            ResponseCode::ContactAdmin.into(),
                            "Database failure.",
                        )
                    })?;
                    if jmap_ids_map.remove(&value) {
                        seq_nums_map.insert(value, seq_num);
                        if seq_nums_map.len() == jmap_ids.len() {
                            break;
                        }
                    }
                }
            }

            // Add missing ids
            if add_missing {
                let mut uid_next_key = None;
                for jmap_id in jmap_ids_map {
                    seq_num += 1;
                    db.insert_jmap_id(
                        &mailbox,
                        jmap_id.as_bytes(),
                        uid_next_key.get_or_insert_with(|| serialize_uid_next_key(&mailbox)),
                    )?;
                    seq_nums_map.insert(jmap_id.to_string(), seq_num);
                }
            }

            Ok(jmap_ids
                .into_iter()
                .map(|jmap_id| seq_nums_map.remove(&jmap_id).unwrap())
                .collect())
        })
        .await
    }

    pub async fn uid_to_jmap(
        &self,
        mailbox: Arc<MailboxData>,
        uids: Vec<u32>,
    ) -> Result<Vec<(u32, Option<String>)>, StatusResponse> {
        let db = self.db.clone();
        self.spawn_worker(move || {
            let mut jmap_ids = Vec::with_capacity(uids.len());
            for uid in uids {
                jmap_ids.push((
                    uid,
                    if let Some(jmap_id) = db
                        .get(serialize_key(&mailbox, UID_TO_JMAP, &uid.to_be_bytes()[..]))
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
                ));
            }
            Ok(jmap_ids)
        })
        .await
    }

    pub async fn seqnum_to_jmap(
        &self,
        mailbox: Arc<MailboxData>,
        seq_nums: Vec<u32>,
    ) -> Result<Vec<(u32, Option<String>)>, StatusResponse> {
        let db = self.db.clone();
        self.spawn_worker(move || {
            let prefix = serialize_key_prefix(&mailbox, UID_TO_JMAP);
            let mut seq_num = 0;
            let mut seq_nums_map = HashMap::with_capacity(seq_nums.len());

            for kv_result in db.scan_prefix(&prefix) {
                let (key, value) = kv_result.map_err(|err| {
                    error!("Failed to scan db: {}", err);
                    StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
                })?;
                if key.len() > prefix.len() {
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
                }
            }
            Ok(seq_nums
                .into_iter()
                .map(|seq_num| (seq_num, seq_nums_map.remove(&seq_num)))
                .collect())
        })
        .await
    }

    pub async fn delete_account(&self, account_id: String) -> super::Result<()> {
        let db = self.db.clone();
        self.spawn_worker(move || {
            let mut batch = sled::Batch::default();

            for kv_result in db.scan_prefix(&serialize_key_account_prefix(&account_id)) {
                let (key, _) = kv_result.map_err(|err| {
                    error!("Failed to scan db: {}", err);
                    StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
                })?;
                batch.remove(key);
            }

            db.apply_batch(batch).map_err(|err| {
                error!("Failed to delete batch: {}", err);
                StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
            })?;

            Ok(())
        })
        .await
    }

    pub async fn delete_mailbox(&self, account_id: &str, mailbox_id: &str) -> super::Result<()> {
        let mut prefix = serialize_key_account_prefix(account_id);
        prefix.extend_from_slice(mailbox_id.as_bytes());

        let db = self.db.clone();
        self.spawn_worker(move || {
            let mut batch = sled::Batch::default();

            for kv_result in db.scan_prefix(&prefix) {
                let (key, _) = kv_result.map_err(|err| {
                    error!("Failed to scan db: {}", err);
                    StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
                })?;
                if key.len() > prefix.len() && key[prefix.len()] <= UID_VALIDITY {
                    batch.remove(key);
                }
            }

            db.apply_batch(batch).map_err(|err| {
                error!("Failed to delete batch: {}", err);
                StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
            })?;

            Ok(())
        })
        .await
    }

    pub async fn uids(&self, mailbox: Arc<MailboxData>) -> super::Result<(u32, u32)> {
        let db = self.db.clone();
        self.spawn_worker(move || {
            Ok((
                db.uid_validity(&mailbox).ok_or_else(|| {
                    error!("Failed to generate UID.");
                    StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
                })?,
                db.uid_next(&mailbox).ok_or_else(|| {
                    error!("Failed to generate UID.");
                    StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
                })?,
            ))
        })
        .await
    }

    pub async fn purge_deleted_mailboxes(&self, account: &Account) -> super::Result<()> {
        if account.mailbox_data.is_empty() {
            debug!(
                "No mailboxes found for account '{}', skipping purge.",
                account.account_id
            );
            return Ok(());
        }
        let account_prefix = serialize_key_account_prefix(&account.account_id);
        let mailbox_keys = account
            .mailbox_data
            .keys()
            .map(|id| id.as_bytes().to_vec())
            .collect::<HashSet<_>>();

        let db = self.db.clone();
        self.spawn_worker(move || {
            let mut has_deletions = false;
            let mut batch = sled::Batch::default();

            for kv_result in db.scan_prefix(&account_prefix) {
                let (key, _) = kv_result.map_err(|err| {
                    error!("Failed to scan db: {}", err);
                    StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
                })?;
                let key_part = &key[account_prefix.len()..];
                if let Some(pos) = key_part.iter().position(|&ch| ch <= UID_VALIDITY) {
                    if pos > 0 && !mailbox_keys.contains(&key_part[..pos]) {
                        batch.remove(key);
                        has_deletions = true;
                    }
                }
            }

            if has_deletions {
                db.apply_batch(batch).map_err(|err| {
                    error!("Failed to delete batch: {}", err);
                    StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
                })?;
            }

            Ok(())
        })
        .await
    }

    pub async fn spawn_worker<U, V>(&self, f: U) -> super::Result<V>
    where
        U: FnOnce() -> super::Result<V> + Send + 'static,
        V: Sync + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();

        self.worker_pool.spawn(move || {
            tx.send(f()).ok();
        });

        rx.await.map_err(|e| {
            error!("Await error: {}", e);
            StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Server error.")
        })?
    }
}

trait ImapUtils {
    fn insert_jmap_id(
        &self,
        mailbox: &MailboxData,
        jmap_id: &[u8],
        uid_next_key: &[u8],
    ) -> Result<sled::IVec, StatusResponse>;
    fn uid_next(&self, mailbox: &MailboxData) -> Option<u32>;
    fn uid_validity(&self, mailbox: &MailboxData) -> Option<u32>;
}

impl ImapUtils for sled::Db {
    fn insert_jmap_id(
        &self,
        mailbox: &MailboxData,
        jmap_id: &[u8],
        uid_next_key: &[u8],
    ) -> Result<sled::IVec, StatusResponse> {
        // Obtain next UID.
        let uid = self
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
            self.insert(serialize_key(mailbox, JMAP_TO_UID, jmap_id), &uid),
            self.insert(serialize_key(mailbox, UID_TO_JMAP, &uid), jmap_id),
        ] {
            result.map_err(|err| {
                error!("Failed to insert key: {}", err);
                StatusResponse::no(None, ResponseCode::ContactAdmin.into(), "Database failure.")
            })?;
        }
        Ok(uid)
    }

    fn uid_next(&self, mailbox: &MailboxData) -> Option<u32> {
        if let Some(bytes) = self.get(serialize_uid_next_key(mailbox)).ok()? {
            (u32::from_be_bytes((&bytes[..]).try_into().ok()?) + 1).into()
        } else {
            0.into()
        }
    }

    fn uid_validity(&self, mailbox: &MailboxData) -> Option<u32> {
        if let Some(bytes) = self.get(serialize_uid_validity_key(mailbox)).ok()? {
            u32::from_be_bytes((&bytes[..]).try_into().ok()?).into()
        } else {
            0.into()
        }
    }
}

fn serialize_key(mailbox: &MailboxData, separator: u8, value: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(
        mailbox.account_id.len()
            + mailbox.mailbox_id.as_ref().map_or(0, |m| m.len())
            + value.len()
            + 2,
    );
    buf.extend_from_slice(mailbox.account_id.as_bytes());
    buf.push(0);
    if let Some(mailbox_id) = mailbox.mailbox_id.as_ref() {
        buf.extend_from_slice(mailbox_id.as_bytes());
    }
    buf.push(separator);
    buf.extend_from_slice(value);
    buf
}

fn serialize_key_prefix(mailbox: &MailboxData, separator: u8) -> Vec<u8> {
    let mut buf = Vec::with_capacity(
        mailbox.account_id.len() + mailbox.mailbox_id.as_ref().map_or(0, |m| m.len()) + 2,
    );
    buf.extend_from_slice(mailbox.account_id.as_bytes());
    buf.push(0);
    if let Some(mailbox_id) = mailbox.mailbox_id.as_ref() {
        buf.extend_from_slice(mailbox_id.as_bytes());
    }
    buf.push(separator);
    buf
}

fn serialize_key_account_prefix(account_id: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(account_id.len() + 1);
    buf.extend_from_slice(account_id.as_bytes());
    buf.push(0);
    buf
}

fn serialize_uid_next_key(mailbox: &MailboxData) -> Vec<u8> {
    let mut buf = Vec::with_capacity(
        mailbox.account_id.len() + mailbox.mailbox_id.as_ref().map_or(0, |m| m.len()) + 2,
    );
    buf.extend_from_slice(mailbox.account_id.as_bytes());
    buf.push(0);
    if let Some(mailbox_id) = mailbox.mailbox_id.as_ref() {
        buf.extend_from_slice(mailbox_id.as_bytes());
    }
    buf.push(UID_NEXT);
    buf
}

fn serialize_uid_validity_key(mailbox: &MailboxData) -> Vec<u8> {
    let mut buf = Vec::with_capacity(
        mailbox.account_id.len() + mailbox.mailbox_id.as_ref().map_or(0, |m| m.len()) + 2,
    );
    buf.extend_from_slice(mailbox.account_id.as_bytes());
    buf.push(0);
    if let Some(mailbox_id) = mailbox.mailbox_id.as_ref() {
        buf.extend_from_slice(mailbox_id.as_bytes());
    }
    buf.push(UID_VALIDITY);
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

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, HashMap},
        sync::Arc,
    };

    use crate::{
        core::{
            config::load_config,
            mailbox::{Account, Mailbox},
        },
        tests::init_settings,
    };

    use super::MailboxData;

    #[tokio::test]
    async fn synchronize_messages() {
        let (settings, temp_dir) = init_settings(true);
        let core = load_config(&settings);

        // Initial test data
        let mailbox = Arc::new(MailboxData {
            account_id: "john".to_string(),
            mailbox_id: "inbox_id".to_string().into(),
        });
        let mailbox_abc = Arc::new(MailboxData {
            account_id: "abc".to_string(),
            mailbox_id: "inbox_id".to_string().into(),
        });
        let mailbox_xyz = Arc::new(MailboxData {
            account_id: "xyz".to_string(),
            mailbox_id: "inbox_id".to_string().into(),
        });
        let jmap_ids = vec![
            "a00".to_string(),
            "b01".to_string(),
            "c02".to_string(),
            "d03".to_string(),
            "e04".to_string(),
            "f05".to_string(),
            "g06".to_string(),
            "h07".to_string(),
            "i08".to_string(),
            "j09".to_string(),
        ];
        let uids = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let seqnums = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

        // Insert test data
        let (_, uid_next) = core
            .update_uids(mailbox.clone(), jmap_ids.clone())
            .await
            .unwrap();
        assert_eq!(uid_next, 10);

        let (_, uid_next) = core
            .update_uids(mailbox_abc.clone(), jmap_ids.clone())
            .await
            .unwrap();
        assert_eq!(uid_next, 10);

        let (_, uid_next) = core
            .update_uids(mailbox_xyz.clone(), jmap_ids.clone())
            .await
            .unwrap();
        assert_eq!(uid_next, 10);

        // Check generated UIDs
        assert_eq!(
            core.jmap_to_uid(mailbox.clone(), jmap_ids.clone(), false)
                .await
                .unwrap(),
            uids
        );
        assert_eq!(
            core.jmap_to_seqnum(
                mailbox.clone(),
                jmap_ids.iter().rev().cloned().collect(),
                false
            )
            .await
            .unwrap(),
            seqnums.iter().rev().cloned().collect::<Vec<_>>()
        );
        assert_eq!(
            core.uid_to_jmap(mailbox.clone(), uids.clone())
                .await
                .unwrap(),
            uids.iter()
                .zip(jmap_ids.iter())
                .map(|(uid, id)| (*uid, id.to_string().into()))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            core.seqnum_to_jmap(mailbox.clone(), seqnums.clone())
                .await
                .unwrap(),
            seqnums
                .iter()
                .zip(jmap_ids.iter())
                .map(|(uid, id)| (*uid, id.to_string().into()))
                .collect::<Vec<_>>()
        );

        // Remove account
        core.delete_account("abc".to_string()).await.unwrap();
        let (uid_validity, uid_next) = core.uids(mailbox_abc.clone()).await.unwrap();
        assert_eq!(uid_validity, 0);
        assert_eq!(uid_next, 0);
        assert_eq!(
            core.seqnum_to_jmap(mailbox_abc.clone(), vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10])
                .await
                .unwrap(),
            vec![
                (1, None),
                (2, None),
                (3, None),
                (4, None),
                (5, None),
                (6, None),
                (7, None),
                (8, None),
                (9, None),
                (10, None),
            ]
        );

        // Remove and add messages
        let jmap_ids = [
            "a00", "b01", "c02", "h07", "i08", "j09", "h10", "i11", "j12", "k13",
        ]
        .into_iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>();
        let uids = vec![0, 1, 2, 7, 8, 9, 10, 11, 12, 13];

        let (_, uid_next) = core
            .update_uids(mailbox.clone(), jmap_ids.clone())
            .await
            .unwrap();
        assert_eq!(uid_next, 14);

        // Check IDs
        assert_eq!(
            core.jmap_to_uid(mailbox.clone(), jmap_ids.clone(), false)
                .await
                .unwrap(),
            uids
        );
        assert_eq!(
            core.jmap_to_seqnum(
                mailbox.clone(),
                jmap_ids.iter().rev().cloned().collect(),
                false
            )
            .await
            .unwrap(),
            seqnums.iter().rev().cloned().collect::<Vec<_>>()
        );
        assert_eq!(
            core.uid_to_jmap(mailbox.clone(), uids.clone())
                .await
                .unwrap(),
            uids.iter()
                .zip(jmap_ids.iter())
                .map(|(uid, id)| (*uid, id.to_string().into()))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            core.seqnum_to_jmap(mailbox.clone(), seqnums.clone())
                .await
                .unwrap(),
            seqnums
                .iter()
                .zip(jmap_ids.iter())
                .map(|(uid, id)| (*uid, id.to_string().into()))
                .collect::<Vec<_>>()
        );

        // Non existant UIDs
        assert_eq!(
            core.uid_to_jmap(mailbox.clone(), vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9])
                .await
                .unwrap(),
            vec![
                (0, Some("a00".to_string())),
                (1, Some("b01".to_string())),
                (2, Some("c02".to_string())),
                (3, None),
                (4, None),
                (5, None),
                (6, None),
                (7, Some("h07".to_string())),
                (8, Some("i08".to_string())),
                (9, Some("j09".to_string()))
            ]
        );
        assert_eq!(
            core.seqnum_to_jmap(mailbox.clone(), vec![10, 11, 1, 12])
                .await
                .unwrap(),
            vec![
                (10, Some("k13".to_string())),
                (11, None),
                (1, Some("a00".to_string())),
                (12, None)
            ]
        );

        // Remove all ids and add some new ids later
        let (_, uid_next) = core.update_uids(mailbox.clone(), vec![]).await.unwrap();
        assert_eq!(uid_next, 14);
        assert_eq!(
            core.uid_to_jmap(mailbox.clone(), vec![0, 7, 14])
                .await
                .unwrap(),
            vec![(0, None), (7, None), (14, None),]
        );
        assert_eq!(
            core.uid_to_jmap(mailbox.clone(), vec![1, 5, 10])
                .await
                .unwrap(),
            vec![(1, None), (5, None), (10, None),]
        );
        let (_, uid_next) = core
            .update_uids(mailbox.clone(), vec!["x01".to_string(), "y02".to_string()])
            .await
            .unwrap();
        assert_eq!(uid_next, 16);
        assert_eq!(
            core.uid_to_jmap(mailbox.clone(), vec![14, 15])
                .await
                .unwrap(),
            vec![(14, Some("x01".to_string())), (15, Some("y02".to_string())),]
        );
        assert_eq!(
            core.seqnum_to_jmap(mailbox.clone(), vec![1, 2, 3])
                .await
                .unwrap(),
            vec![
                (1, Some("x01".to_string())),
                (2, Some("y02".to_string())),
                (3, None),
            ]
        );

        // Test mailbox purge
        let mailbox_2 = Arc::new(MailboxData {
            account_id: "john".to_string(),
            mailbox_id: "folder_id".to_string().into(),
        });
        let jmap_ids = vec![
            "a00".to_string(),
            "b01".to_string(),
            "c02".to_string(),
            "d03".to_string(),
            "e04".to_string(),
            "f05".to_string(),
            "g06".to_string(),
            "h07".to_string(),
            "i08".to_string(),
            "j09".to_string(),
        ];
        let uids = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let (_, uid_next) = core
            .update_uids(mailbox_2.clone(), jmap_ids.clone())
            .await
            .unwrap();
        assert_eq!(uid_next, 10);

        core.purge_deleted_mailboxes(&Account {
            account_id: "john".to_string(),
            state_id: String::new(),
            prefix: None,
            mailbox_names: BTreeMap::new(),
            mailbox_data: HashMap::from_iter([("folder_id".to_string(), Mailbox::default())]),
        })
        .await
        .unwrap();
        let (uid_validity, uid_next) = core.uids(mailbox.clone()).await.unwrap();
        assert_eq!(uid_validity, 0);
        assert_eq!(uid_next, 0);

        assert_eq!(
            core.uid_to_jmap(mailbox_2.clone(), uids.clone())
                .await
                .unwrap(),
            uids.iter()
                .zip(jmap_ids.iter())
                .map(|(uid, id)| (*uid, id.to_string().into()))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            core.seqnum_to_jmap(mailbox.clone(), vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10])
                .await
                .unwrap(),
            vec![
                (1, None),
                (2, None),
                (3, None),
                (4, None),
                (5, None),
                (6, None),
                (7, None),
                (8, None),
                (9, None),
                (10, None),
            ]
        );

        // Delete temporary directory
        if temp_dir.exists() {
            std::fs::remove_dir_all(&temp_dir).unwrap();
        }
    }
}
