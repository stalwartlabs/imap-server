use super::Sequence;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arguments {
    pub sequence_set: Sequence,
    pub mailbox_name: String,
}
