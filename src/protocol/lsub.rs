use crate::core::StatusResponse;

use super::{list::ListItem, ImapResponse, ProtocolVersion};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub list_items: Vec<ListItem>,
}

impl ImapResponse for Response {
    fn serialize(&self, tag: String, version: ProtocolVersion) -> Vec<u8> {
        let mut buf = Vec::with_capacity(100);
        for list_item in &self.list_items {
            list_item.serialize(&mut buf, version, true);
        }

        StatusResponse::ok(tag.into(), None, "completed").serialize(&mut buf);
        buf
    }
}
