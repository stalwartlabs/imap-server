use jmap_client::mailbox::Role;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arguments {
    pub tag: String,
    pub mailbox_name: String,
    pub mailbox_role: Role,
}
