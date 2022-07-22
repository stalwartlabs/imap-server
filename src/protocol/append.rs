use crate::core::Flag;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arguments {
    pub tag: String,
    pub mailbox_name: String,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub message: Vec<u8>,
    pub flags: Vec<Flag>,
    pub received_at: Option<i64>,
}
