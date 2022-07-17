use crate::core::{Command, Flag, StatusResponse};

use super::{fetch::FetchItem, ImapResponse, Sequence};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arguments {
    pub tag: String,
    pub sequence_set: Sequence,
    pub operation: Operation,
    pub is_silent: bool,
    pub keywords: Vec<Flag>,
    pub unchanged_since: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    Set,
    Add,
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response<'x> {
    pub is_uid: bool,
    pub items: Vec<FetchItem<'x>>,
}

impl<'x> ImapResponse for Response<'x> {
    fn serialize(&self, tag: String) -> Vec<u8> {
        let mut buf = Vec::with_capacity(64);
        for item in &self.items {
            item.serialize(&mut buf);
        }
        StatusResponse::completed(Command::Store(self.is_uid), tag).serialize(&mut buf);
        buf
    }
}
