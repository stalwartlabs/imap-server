use crate::core::{Flag, StatusResponse};

use super::{fetch::FetchItem, ImapResponse, Sequence};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arguments {
    pub tag: String,
    pub sequence_set: Sequence,
    pub operation: Operation,
    pub is_silent: bool,
    pub keywords: Vec<Flag>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    Set,
    Add,
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub items: Vec<FetchItem>,
}

impl ImapResponse for Response {
    fn serialize(&self, tag: String, _version: super::ProtocolVersion) -> Vec<u8> {
        let mut buf = Vec::with_capacity(64);
        for item in &self.items {
            item.serialize(&mut buf);
        }
        StatusResponse::ok(tag.into(), None, "STORE completed").serialize(&mut buf);
        buf
    }
}
