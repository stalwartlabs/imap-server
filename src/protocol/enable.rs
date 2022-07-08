use super::capability::Capability;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arguments {
    pub tag: String,
    pub capabilities: Vec<Capability>,
}
