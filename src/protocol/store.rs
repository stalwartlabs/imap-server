use crate::core::Flag;

use super::Sequence;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arguments {
    pub sequence_set: Vec<Sequence>,
    pub operation: Operation,
    pub keywords: Vec<Flag>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    Set,
    SetSilent,
    Add,
    AddSilent,
    Clear,
    ClearSilent,
}
