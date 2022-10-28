pub mod client;
pub mod connection;
pub mod listener;

use std::borrow::Cow;

use crate::core::receiver::CommandParser;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Authenticate,
    StartTls,
    Logout,
    Capability,
    HaveSpace,
    PutScript,
    ListScripts,
    SetActive,
    GetScript,
    DeleteScript,
    RenameScript,
    CheckScript,
    Noop,
    Unauthenticate,
}

impl CommandParser for Command {
    fn parse(value: &[u8], _is_uid: bool) -> Option<Self> {
        match value {
            b"AUTHENTICATE" => Some(Command::Authenticate),
            b"STARTTLS" => Some(Command::StartTls),
            b"LOGOUT" => Some(Command::Logout),
            b"CAPABILITY" => Some(Command::Capability),
            b"HAVESPACE" => Some(Command::HaveSpace),
            b"PUTSCRIPT" => Some(Command::PutScript),
            b"LISTSCRIPTS" => Some(Command::ListScripts),
            b"SETACTIVE" => Some(Command::SetActive),
            b"GETSCRIPT" => Some(Command::GetScript),
            b"DELETESCRIPT" => Some(Command::DeleteScript),
            b"RENAMESCRIPT" => Some(Command::RenameScript),
            b"CHECKSCRIPT" => Some(Command::CheckScript),
            b"NOOP" => Some(Command::Noop),
            b"UNAUTHENTICATE" => Some(Command::Unauthenticate),
            _ => None,
        }
    }

    fn tokenize_brackets(&self) -> bool {
        false
    }
}

impl Default for Command {
    fn default() -> Self {
        Command::Noop
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusResponse {
    pub code: Option<ResponseCode>,
    pub message: Cow<'static, str>,
    pub rtype: ResponseType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseType {
    Ok,
    No,
    Bye,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseCode {
    AuthTooWeak,
    EncryptNeeded,
    Quota,
    QuotaMaxScripts,
    QuotaMaxSize,
    Referral,
    Sasl,
    TransitionNeeded,
    TryLater,
    Active,
    NonExistent,
    AlreadyExists,
    Tag,
    Warnings,
}

impl ResponseCode {
    pub fn serialize(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(match self {
            ResponseCode::AuthTooWeak => b"AUTH-TOO-WEAK",
            ResponseCode::EncryptNeeded => b"ENCRYPT-NEEDED",
            ResponseCode::Quota => b"QUOTA",
            ResponseCode::QuotaMaxScripts => b"QUOTA/MAXSCRIPTS",
            ResponseCode::QuotaMaxSize => b"QUOTA/MAXSIZE",
            ResponseCode::Referral => b"REFERRAL",
            ResponseCode::Sasl => b"SASL",
            ResponseCode::TransitionNeeded => b"TRANSITION-NEEDED",
            ResponseCode::TryLater => b"TRYLATER",
            ResponseCode::Active => b"ACTIVE",
            ResponseCode::NonExistent => b"NONEXISTENT",
            ResponseCode::AlreadyExists => b"ALREADYEXISTS",
            ResponseCode::Tag => b"TAG",
            ResponseCode::Warnings => b"WARNINGS",
        });
    }
}

impl ResponseType {
    pub fn serialize(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(match self {
            ResponseType::Ok => b"OK",
            ResponseType::No => b"NO",
            ResponseType::Bye => b"BYE",
        });
    }
}

impl StatusResponse {
    pub fn serialize(self, mut buf: Vec<u8>) -> Vec<u8> {
        self.rtype.serialize(&mut buf);
        if let Some(code) = &self.code {
            buf.extend_from_slice(b" (");
            code.serialize(&mut buf);
            buf.push(b')');
        }
        if !self.message.is_empty() {
            buf.push(b' ');
            buf.extend_from_slice(self.message.as_bytes());
        }
        buf.extend_from_slice(b"\r\n");
        buf
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.serialize(Vec::with_capacity(16))
    }

    pub fn with_code(mut self, code: ResponseCode) -> Self {
        self.code = Some(code);
        self
    }

    pub fn no(message: impl Into<Cow<'static, str>>) -> Self {
        StatusResponse {
            code: None,
            message: message.into(),
            rtype: ResponseType::No,
        }
    }

    pub fn ok(message: impl Into<Cow<'static, str>>) -> Self {
        StatusResponse {
            code: None,
            message: message.into(),
            rtype: ResponseType::Ok,
        }
    }

    pub fn bye(message: impl Into<Cow<'static, str>>) -> Self {
        StatusResponse {
            code: None,
            message: message.into(),
            rtype: ResponseType::Bye,
        }
    }
}
