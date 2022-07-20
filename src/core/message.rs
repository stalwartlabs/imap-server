use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::SystemTime,
};

use jmap_client::email::query::Filter;
use tokio::sync::oneshot;
use tracing::{debug, error};

use crate::protocol::Sequence;

use super::{client::SessionData, mailbox::Account, Core, IntoStatusResponse, StatusResponse};

pub struct MailboxData {
    pub account_id: String,
    pub mailbox_id: Option<String>,
}

#[derive(Debug)]
pub struct MailboxStatus {
    pub uid_next: u32,
    pub uid_validity: u32,
    pub total_messages: usize,
}

#[derive(Debug, Default)]
pub struct IdMappings {
    pub jmap_ids: Vec<String>,
    pub uids: Vec<u32>,
    pub seqnums: Option<Vec<u32>>,
}

pub const JMAP_TO_UID: u8 = 0;
pub const UID_TO_JMAP: u8 = 1;
pub const UID_NEXT: u8 = 2;
pub const UID_VALIDITY: u8 = 3;
pub const MODSEQ_TO_STATE: u8 = 4;
pub const STATE_TO_MODSEQ: u8 = 5;
pub const HIGHEST_MODSEQ: u8 = 6;
pub const JMAP_DELETED_IDS: u8 = 7;

impl SessionData {
    pub async fn synchronize_messages(
        &self,
        mailbox: Arc<MailboxData>,
    ) -> Result<MailboxStatus, StatusResponse> {
        let mut valid_ids = Vec::new();
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
                .map_err(|err| err.into_status_response())?;
            total_messages = response.total().unwrap_or(0);
            let emails = response.take_ids();

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
        let update_result = self
            .core
            .update_uids(mailbox, valid_ids)
            .await
            .map_err(|_| StatusResponse::database_failure())?;

        Ok(MailboxStatus {
            uid_next: update_result.uid_next,
            uid_validity: update_result.uid_validity,
            total_messages,
        })
    }

    pub async fn synchronize_state(&self, account_id: &str) -> Result<u32, StatusResponse> {
        let mut request = self.client.build();
        request
            .get_email()
            .account_id(account_id)
            .ids(Vec::<&str>::new());
        let mut response = request
            .send_get_email()
            .await
            .map_err(|err| err.into_status_response())?;

        // Update modseq
        self.core
            .state_to_modseq(account_id, response.take_state())
            .await
            .map_err(|_| StatusResponse::database_failure())
    }

    pub async fn imap_sequence_to_jmap(
        &self,
        mailbox: Arc<MailboxData>,
        sequence: Sequence,
        is_uid: bool,
    ) -> crate::core::Result<Arc<IdMappings>> {
        if sequence != Sequence::SavedSearch {
            self.core
                .imap_sequence_to_jmap(mailbox, sequence, is_uid)
                .await
                .map(Arc::new)
                .map_err(|_| StatusResponse::database_failure())
        } else {
            self.get_saved_search()
                .await
                .ok_or_else(|| StatusResponse::no("No saved search found."))
        }
    }
}

#[derive(Debug)]
pub struct UpdateResult {
    pub uid_validity: u32,
    pub uid_next: u32,
}

impl Core {
    pub async fn update_uids(
        &self,
        mailbox: Arc<MailboxData>,
        jmap_ids: Vec<String>,
    ) -> Result<UpdateResult, ()> {
        let db = self.db.clone();
        self.spawn_worker(move || {
            // Obtain/generate UIDVALIDITY
            let uid_validity = db.uid_validity(&mailbox)?;

            // Remove from cache messages no longer present in the mailbox.
            let mut jmap_ids_map = jmap_ids
                .iter()
                .map(|id| id.as_bytes())
                .collect::<HashSet<_>>();

            let prefix = serialize_key_prefix(&mailbox, JMAP_TO_UID);
            let mut batch = sled::Batch::default();
            let mut has_deletions = false;
            let mut seq_num: u32 = 0;

            for kv_result in db.scan_prefix(&prefix) {
                let (key, value) = kv_result.map_err(|err| {
                    error!("Failed to scan db: {}", err);
                })?;
                if key.len() > prefix.len() {
                    seq_num += 1;
                    let jmap_id = &key[prefix.len()..];
                    if !jmap_ids_map.remove(jmap_id) {
                        // Add UID and seqnum to deleted messages
                        let mut buf = Vec::with_capacity(
                            (key.len() - prefix.len())
                                + std::mem::size_of::<u64>()
                                + std::mem::size_of::<u32>(),
                        );
                        buf.extend_from_slice(&value);
                        buf.extend_from_slice(&seq_num.to_be_bytes());
                        buf.extend_from_slice(
                            &SystemTime::now()
                                .duration_since(SystemTime::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0)
                                .to_be_bytes(),
                        );
                        batch.insert(serialize_key(&mailbox, JMAP_DELETED_IDS, jmap_id), buf);

                        // Delete mappings from cache
                        batch.remove(key);
                        batch.remove(sled::IVec::from(serialize_key(
                            &mailbox,
                            UID_TO_JMAP,
                            &value,
                        )));

                        //TODO: Delete old deleted IDs after X days

                        has_deletions = true;
                    }
                }
            }

            if has_deletions {
                db.apply_batch(batch).map_err(|err| {
                    error!("Failed to delete batch: {}", err);
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

            Ok(UpdateResult {
                uid_validity,
                uid_next: db.uid_next(&mailbox)?,
            })
        })
        .await
    }

    pub async fn jmap_to_imap(
        &self,
        mailbox: Arc<MailboxData>,
        jmap_ids: Vec<String>,
        add_missing: bool,
        as_uid: bool,
    ) -> Result<IdMappings, ()> {
        if jmap_ids.is_empty() {
            return Ok(IdMappings::default());
        }

        let db = self.db.clone();
        self.spawn_worker(move || {
            if as_uid {
                let mut uids = Vec::with_capacity(jmap_ids.len());
                let mut uid_next_key = None;
                for jmap_id in &jmap_ids {
                    let jmap_id = jmap_id.as_bytes();
                    let uid = if let Some(uid) = db
                        .get(serialize_key(&mailbox, JMAP_TO_UID, jmap_id))
                        .map_err(|err| {
                            error!("Failed to get key: {}", err);
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
                    })?));
                }

                Ok(IdMappings {
                    jmap_ids,
                    uids,
                    seqnums: None,
                })
            } else {
                let prefix = serialize_key_prefix(&mailbox, UID_TO_JMAP);
                let mut jmap_ids_map = jmap_ids.iter().collect::<HashSet<_>>();
                let mut seq_num = 0;
                let mut seq_nums_map = HashMap::with_capacity(jmap_ids.len());
                let mut uids = Vec::with_capacity(jmap_ids.len());

                for kv_result in db.scan_prefix(&prefix) {
                    let (key, value) = kv_result.map_err(|err| {
                        error!("Failed to scan db: {}", err);
                    })?;
                    if key.len() > prefix.len() {
                        seq_num += 1;

                        let value = String::from_utf8(value.to_vec()).map_err(|_| {
                            error!("Failed to convert bytes to string.");
                        })?;
                        if jmap_ids_map.remove(&value) {
                            uids.push(u32::from_be_bytes(
                                (&key[prefix.len()..]).try_into().map_err(|_| {
                                    error!("Failed to convert bytes to u32.");
                                })?,
                            ));
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
                        let uid = db.insert_jmap_id(
                            &mailbox,
                            jmap_id.as_bytes(),
                            uid_next_key.get_or_insert_with(|| serialize_uid_next_key(&mailbox)),
                        )?;
                        uids.push(u32::from_be_bytes((&uid[..]).try_into().map_err(|_| {
                            error!("Failed to convert bytes to u32.");
                        })?));
                        seq_nums_map.insert(jmap_id.to_string(), seq_num);
                    }
                }

                Ok(IdMappings {
                    uids,
                    seqnums: jmap_ids
                        .iter()
                        .filter_map(|jmap_id| seq_nums_map.remove(jmap_id))
                        .collect::<Vec<_>>()
                        .into(),
                    jmap_ids,
                })
            }
        })
        .await
    }

    pub async fn jmap_deletions_to_imap(
        &self,
        mailbox: Arc<MailboxData>,
        jmap_ids: Vec<String>,
        as_uid: bool,
    ) -> Result<Vec<u32>, ()> {
        if jmap_ids.is_empty() {
            return Ok(Vec::new());
        }

        let db = self.db.clone();
        self.spawn_worker(move || {
            let mut uids = Vec::with_capacity(jmap_ids.len());

            for jmap_id in &jmap_ids {
                let jmap_id = jmap_id.as_bytes();
                let uid = if let Some(uid) = db
                    .get(serialize_key(&mailbox, JMAP_TO_UID, jmap_id))
                    .map_err(|err| {
                    error!("Failed to get key: {}", err);
                })? {
                    u32::from_be_bytes((&uid[..]).try_into().map_err(|_| {
                        error!("Failed to convert bytes to u32.");
                    })?)
                } else if let Some(uid) = db
                    .get(serialize_key(&mailbox, JMAP_DELETED_IDS, jmap_id))
                    .map_err(|err| {
                        error!("Failed to get key: {}", err);
                    })?
                {
                    u32::from_be_bytes(
                        (if as_uid {
                            &uid[..std::mem::size_of::<u32>()]
                        } else {
                            &uid[std::mem::size_of::<u32>()..(std::mem::size_of::<u32>() * 2)]
                        })
                        .try_into()
                        .map_err(|_| {
                            error!("Failed to convert bytes to u32.");
                        })?,
                    )
                } else {
                    continue;
                };

                uids.push(uid);
            }

            Ok(uids)
        })
        .await
    }

    pub async fn imap_to_jmap(
        &self,
        mailbox: Arc<MailboxData>,
        imap_ids: Vec<u32>,
        is_uid: bool,
    ) -> Result<IdMappings, ()> {
        let db = self.db.clone();
        self.spawn_worker(move || {
            let mut jmap_ids = Vec::with_capacity(imap_ids.len());
            if is_uid {
                let mut uids = Vec::with_capacity(imap_ids.len());
                for uid in imap_ids {
                    if let Some(jmap_id) = db
                        .get(serialize_key(&mailbox, UID_TO_JMAP, &uid.to_be_bytes()[..]))
                        .map_err(|err| {
                            error!("Failed to get key: {}", err);
                        })?
                    {
                        uids.push(uid);
                        jmap_ids.push(String::from_utf8(jmap_id.to_vec()).map_err(|_| {
                            error!("Failed to convert bytes to string.");
                        })?);
                    }
                }
                Ok(IdMappings {
                    jmap_ids,
                    uids,
                    seqnums: None,
                })
            } else {
                let prefix = serialize_key_prefix(&mailbox, UID_TO_JMAP);
                let mut uids = Vec::with_capacity(imap_ids.len());
                let mut seqnums = Vec::with_capacity(imap_ids.len());
                let mut seq_num = 0;

                for kv_result in db.scan_prefix(&prefix) {
                    let (key, value) = kv_result.map_err(|err| {
                        error!("Failed to scan db: {}", err);
                    })?;
                    if key.len() > prefix.len() {
                        seq_num += 1;

                        if imap_ids.contains(&seq_num) {
                            seqnums.push(seq_num);
                            uids.push(u32::from_be_bytes(
                                (&key[prefix.len()..]).try_into().map_err(|_| {
                                    error!("Failed to convert bytes to u32.");
                                })?,
                            ));
                            jmap_ids.push(String::from_utf8(value.to_vec()).map_err(|_| {
                                error!("Failed to convert bytes to string.");
                            })?);
                            if jmap_ids.len() == imap_ids.len() {
                                break;
                            }
                        }
                    }
                }

                Ok(IdMappings {
                    jmap_ids,
                    uids,
                    seqnums: seqnums.into(),
                })
            }
        })
        .await
    }

    pub async fn imap_sequence_to_jmap(
        &self,
        mailbox: Arc<MailboxData>,
        sequence: Sequence,
        is_uid: bool,
    ) -> Result<IdMappings, ()> {
        if let Some(ids) = sequence.try_expand() {
            return self.imap_to_jmap(mailbox, ids, is_uid).await;
        }

        let db = self.db.clone();
        self.spawn_worker(move || {
            let prefix = serialize_key_prefix(&mailbox, UID_TO_JMAP);
            let mut seq_num = 0;
            let mut jmap_ids = Vec::new();
            let mut uids = Vec::new();
            let mut seqnums = Vec::new();

            for kv_result in db.scan_prefix(&prefix) {
                let (key, value) = kv_result.map_err(|err| {
                    error!("Failed to scan db: {}", err);
                })?;
                if key.len() > prefix.len() {
                    seq_num += 1;
                    let uid =
                        u32::from_be_bytes((&key[prefix.len()..]).try_into().map_err(|_| {
                            error!("Failed to convert bytes to u32.");
                        })?);

                    if sequence.contains(if is_uid { uid } else { seq_num }) {
                        jmap_ids.push(String::from_utf8(value.to_vec()).map_err(|_| {
                            error!("Failed to convert bytes to string.");
                        })?);
                        uids.push(uid);
                        seqnums.push(seq_num);
                    }
                }
            }

            Ok(IdMappings {
                jmap_ids,
                uids,
                seqnums: seqnums.into(),
            })
        })
        .await
    }

    pub async fn delete_account(&self, account_id: String) -> Result<(), ()> {
        let db = self.db.clone();
        self.spawn_worker(move || {
            let mut batch = sled::Batch::default();

            for kv_result in db.scan_prefix(&serialize_key_account_prefix(&account_id)) {
                let (key, _) = kv_result.map_err(|err| {
                    error!("Failed to scan db: {}", err);
                })?;
                batch.remove(key);
            }

            db.apply_batch(batch).map_err(|err| {
                error!("Failed to delete batch: {}", err);
            })?;

            Ok(())
        })
        .await
    }

    pub async fn delete_mailbox(&self, account_id: &str, mailbox_id: &str) -> Result<(), ()> {
        let mut prefix = serialize_key_account_prefix(account_id);
        prefix.extend_from_slice(mailbox_id.as_bytes());

        let db = self.db.clone();
        self.spawn_worker(move || {
            let mut batch = sled::Batch::default();

            for kv_result in db.scan_prefix(&prefix) {
                let (key, _) = kv_result.map_err(|err| {
                    error!("Failed to scan db: {}", err);
                })?;
                if key.len() > prefix.len() && key[prefix.len()] <= UID_VALIDITY {
                    batch.remove(key);
                }
            }

            db.apply_batch(batch).map_err(|err| {
                error!("Failed to delete batch: {}", err);
            })?;

            Ok(())
        })
        .await
    }

    pub async fn uids(&self, mailbox: Arc<MailboxData>) -> Result<(u32, u32), ()> {
        let db = self.db.clone();
        self.spawn_worker(move || Ok((db.uid_validity(&mailbox)?, db.uid_next(&mailbox)?)))
            .await
    }

    pub async fn purge_deleted_mailboxes(&self, account: &Account) -> Result<(), ()> {
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
                })?;
            }

            Ok(())
        })
        .await
    }

    pub async fn spawn_worker<U, V>(&self, f: U) -> Result<V, ()>
    where
        U: FnOnce() -> Result<V, ()> + Send + 'static,
        V: Sync + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();

        self.worker_pool.spawn(move || {
            tx.send(f()).ok();
        });

        rx.await.map_err(|e| {
            error!("Await error: {}", e);
        })?
    }
}

trait ImapUtils {
    fn insert_jmap_id(
        &self,
        mailbox: &MailboxData,
        jmap_id: &[u8],
        uid_next_key: &[u8],
    ) -> Result<sled::IVec, ()>;
    fn uid_next(&self, mailbox: &MailboxData) -> Result<u32, ()>;
    fn uid_validity(&self, mailbox: &MailboxData) -> Result<u32, ()>;
}

impl ImapUtils for sled::Db {
    fn insert_jmap_id(
        &self,
        mailbox: &MailboxData,
        jmap_id: &[u8],
        uid_next_key: &[u8],
    ) -> Result<sled::IVec, ()> {
        // Obtain next UID.
        let uid = self
            .update_and_fetch(&uid_next_key, increment_uid)
            .map_err(|err| {
                error!("Failed to increment UID: {}", err);
            })?
            .ok_or_else(|| {
                error!("Failed to generate UID.");
            })?;

        // Write keys
        for result in [
            self.insert(serialize_key(mailbox, JMAP_TO_UID, jmap_id), &uid),
            self.insert(serialize_key(mailbox, UID_TO_JMAP, &uid), jmap_id),
        ] {
            result.map_err(|err| {
                error!("Failed to insert key: {}", err);
            })?;
        }
        Ok(uid)
    }

    fn uid_next(&self, mailbox: &MailboxData) -> Result<u32, ()> {
        Ok(
            if let Some(uid_bytes) = self.get(serialize_uid_next_key(mailbox)).map_err(|err| {
                error!("Failed to read key: {}", err);
            })? {
                u32::from_be_bytes((&uid_bytes[..]).try_into().map_err(|err| {
                    error!("Failed to decode UID next: {}", err);
                })?) + 1
            } else {
                0
            },
        )
    }

    fn uid_validity(&self, mailbox: &MailboxData) -> Result<u32, ()> {
        // Obtain/generate UIDVALIDITY
        let uid_validity_key = serialize_uid_validity_key(mailbox);
        Ok(
            if let Some(uid_bytes) = self.get(&uid_validity_key).map_err(|err| {
                error!("Failed to read key: {}", err);
            })? {
                u32::from_be_bytes((&uid_bytes[..]).try_into().map_err(|err| {
                    error!("Failed to decode UID validity: {}", err);
                })?)
            } else {
                // Number of hours since January 1st, 2000
                let uid_validity = (SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
                    .saturating_sub(946684800)
                    / 3600) as u32;
                self.insert(uid_validity_key, &uid_validity.to_be_bytes()[..])
                    .map_err(|err| {
                        error!("Failed to insert key: {}", err);
                    })?;
                uid_validity
            },
        )
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

pub fn serialize_modseq(account_id: &[u8], value: &[u8], separator: u8) -> Vec<u8> {
    let mut buf = Vec::with_capacity(account_id.len() + value.len() + 2);
    buf.extend_from_slice(account_id);
    buf.push(0);
    buf.extend_from_slice(value);
    buf.push(separator);
    buf
}

pub fn serialize_highestmodseq(account_id: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(account_id.len() + 2);
    buf.extend_from_slice(account_id);
    buf.push(0);
    buf.push(HIGHEST_MODSEQ);
    buf
}

pub fn increment_uid(old: Option<&[u8]>) -> Option<Vec<u8>> {
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
        let update_result = core
            .update_uids(mailbox.clone(), jmap_ids.clone())
            .await
            .unwrap();
        assert_eq!(update_result.uid_next, 10);

        let update_result = core
            .update_uids(mailbox_abc.clone(), jmap_ids.clone())
            .await
            .unwrap();
        assert_eq!(update_result.uid_next, 10);

        let update_result = core
            .update_uids(mailbox_xyz.clone(), jmap_ids.clone())
            .await
            .unwrap();
        assert_eq!(update_result.uid_next, 10);

        // Check generated UIDs
        assert_eq!(
            core.jmap_to_imap(mailbox.clone(), jmap_ids.clone(), false, true)
                .await
                .unwrap()
                .uids,
            uids
        );
        assert_eq!(
            core.jmap_to_imap(
                mailbox.clone(),
                jmap_ids.iter().rev().cloned().collect(),
                false,
                false
            )
            .await
            .unwrap()
            .seqnums
            .unwrap(),
            seqnums.iter().rev().cloned().collect::<Vec<_>>()
        );
        assert_eq!(
            core.imap_to_jmap(mailbox.clone(), uids.clone(), true)
                .await
                .unwrap()
                .jmap_ids,
            jmap_ids
        );
        assert_eq!(
            core.imap_to_jmap(mailbox.clone(), seqnums.clone(), false)
                .await
                .unwrap()
                .jmap_ids,
            jmap_ids
        );

        // Remove account
        core.delete_account("abc".to_string()).await.unwrap();
        let (uid_validity, uid_next) = core.uids(mailbox_abc.clone()).await.unwrap();
        assert_eq!(uid_validity, 0);
        assert_eq!(uid_next, 0);
        assert_eq!(
            core.imap_to_jmap(
                mailbox_abc.clone(),
                vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
                false
            )
            .await
            .unwrap()
            .jmap_ids,
            Vec::<String>::new()
        );

        // Remove and add messages
        let jmap_ids = [
            "a00", "b01", "c02", "h07", "i08", "j09", "h10", "i11", "j12", "k13",
        ]
        .into_iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>();
        let uids = vec![0, 1, 2, 7, 8, 9, 10, 11, 12, 13];

        let update_result = core
            .update_uids(mailbox.clone(), jmap_ids.clone())
            .await
            .unwrap();
        assert_eq!(update_result.uid_next, 14);

        // Check IDs
        assert_eq!(
            core.jmap_to_imap(mailbox.clone(), jmap_ids.clone(), false, true)
                .await
                .unwrap()
                .uids,
            uids
        );
        assert_eq!(
            core.jmap_to_imap(
                mailbox.clone(),
                jmap_ids.iter().rev().cloned().collect(),
                false,
                false
            )
            .await
            .unwrap()
            .seqnums
            .unwrap(),
            seqnums.iter().rev().cloned().collect::<Vec<_>>()
        );
        assert_eq!(
            core.imap_to_jmap(mailbox.clone(), uids.clone(), true)
                .await
                .unwrap()
                .jmap_ids,
            jmap_ids
        );
        assert_eq!(
            core.imap_to_jmap(mailbox.clone(), seqnums.clone(), false)
                .await
                .unwrap()
                .jmap_ids,
            jmap_ids
        );

        // Non existant UIDs
        assert_eq!(
            core.imap_to_jmap(mailbox.clone(), vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9], true)
                .await
                .unwrap()
                .jmap_ids,
            vec![
                "a00".to_string(),
                "b01".to_string(),
                "c02".to_string(),
                "h07".to_string(),
                "i08".to_string(),
                "j09".to_string()
            ]
        );
        assert_eq!(
            core.imap_to_jmap(mailbox.clone(), vec![10, 11, 1, 12], false)
                .await
                .unwrap()
                .jmap_ids,
            vec!["a00".to_string(), "k13".to_string()]
        );

        // Remove all ids and add some new ids later
        let update_result = core.update_uids(mailbox.clone(), vec![]).await.unwrap();
        assert_eq!(update_result.uid_next, 14);
        assert_eq!(
            core.imap_to_jmap(mailbox.clone(), vec![0, 7, 14], true)
                .await
                .unwrap()
                .jmap_ids,
            Vec::<String>::new()
        );
        assert_eq!(
            core.imap_to_jmap(mailbox.clone(), vec![1, 5, 10], true)
                .await
                .unwrap()
                .jmap_ids,
            Vec::<String>::new()
        );
        let update_result = core
            .update_uids(mailbox.clone(), vec!["x01".to_string(), "y02".to_string()])
            .await
            .unwrap();
        assert_eq!(update_result.uid_next, 16);
        assert_eq!(
            core.imap_to_jmap(mailbox.clone(), vec![14, 15], true)
                .await
                .unwrap()
                .jmap_ids,
            vec!["x01".to_string(), "y02".to_string(),]
        );
        assert_eq!(
            core.imap_to_jmap(mailbox.clone(), vec![1, 2, 3], false)
                .await
                .unwrap()
                .jmap_ids,
            vec!["x01".to_string(), "y02".to_string()]
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
        let update_result = core
            .update_uids(mailbox_2.clone(), jmap_ids.clone())
            .await
            .unwrap();
        assert_eq!(update_result.uid_next, 10);

        core.purge_deleted_mailboxes(&Account {
            account_id: "john".to_string(),
            prefix: None,
            mailbox_names: BTreeMap::new(),
            mailbox_data: HashMap::from_iter([("folder_id".to_string(), Mailbox::default())]),
            mailbox_state: String::new(),
            modseq: None,
        })
        .await
        .unwrap();
        let (uid_validity, uid_next) = core.uids(mailbox.clone()).await.unwrap();
        assert_eq!(uid_validity, 0);
        assert_eq!(uid_next, 0);

        assert_eq!(
            core.imap_to_jmap(mailbox_2.clone(), uids.clone(), true)
                .await
                .unwrap()
                .jmap_ids,
            jmap_ids
        );
        assert_eq!(
            core.imap_to_jmap(mailbox.clone(), vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10], false)
                .await
                .unwrap()
                .jmap_ids,
            Vec::<String>::new()
        );

        // Delete temporary directory
        if temp_dir.exists() {
            std::fs::remove_dir_all(&temp_dir).unwrap();
        }
    }
}
