use crate::core::Flag;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arguments {
    pub mailbox_name: String,
    pub message: Vec<u8>,
    pub flags: Vec<Flag>,
    pub received_at: Option<i64>,
}
