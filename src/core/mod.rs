pub mod client;
pub mod config;
pub mod connection;
pub mod env_settings;
pub mod listener;
pub mod mailbox;
pub mod message;
pub mod receiver;
pub mod utf7;
pub mod writer;

use std::{borrow::Cow, sync::Arc};

use jmap_client::core::{
    error::{JMAPError, MethodErrorType, ProblemType},
    set::SetErrorType,
};

use crate::protocol::capability::Capability;

pub struct Core {
    pub tls_acceptor: tokio_rustls::TlsAcceptor,
    pub db: Arc<sled::Db>,
    pub worker_pool: rayon::ThreadPool,
    pub jmap_url: String,
    pub folder_shared: String,
    pub folder_all: String,
    pub max_request_size: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    // Client Commands - Any State
    Capability,
    Noop,
    Logout,

    // Client Commands - Not Authenticated State
    StartTls,
    Authenticate,
    Login,

    // Client Commands - Authenticated State
    Enable,
    Select,
    Examine,
    Create,
    Delete,
    Rename,
    Subscribe,
    Unsubscribe,
    List,
    Namespace,
    Status,
    Append,
    Idle,

    // Client Commands - Selected State
    Close,
    Unselect,
    Expunge(bool),
    Search(bool),
    Fetch(bool),
    Store(bool),
    Copy(bool),
    Move(bool),

    // IMAP4rev1
    Lsub,
    Check,

    // RFC 5256
    Sort(bool),
    Thread(bool),

    // RFC 4314
    SetAcl,
    DeleteAcl,
    GetAcl,
    ListRights,
    MyRights,

    // RFC 8437
    Unauthenticate,

    // RFC 2971
    Id,
}

impl Command {
    #[inline(always)]
    pub fn is_fetch(&self) -> bool {
        matches!(self, Command::Fetch(_))
    }

    pub fn is_uid(&self) -> bool {
        matches!(
            self,
            Command::Fetch(true)
                | Command::Search(true)
                | Command::Copy(true)
                | Command::Move(true)
                | Command::Store(true)
                | Command::Expunge(true)
                | Command::Sort(true)
                | Command::Thread(true)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flag {
    Seen,
    Draft,
    Flagged,
    Answered,
    Recent,
    Important,
    Phishing,
    Junk,
    NotJunk,
    Deleted,
    Forwarded,
    MDNSent,
    Keyword(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseCode {
    Alert,
    AlreadyExists,
    AppendUid { uid_validity: u32, uids: Vec<u32> },
    AuthenticationFailed,
    AuthorizationFailed,
    BadCharset,
    Cannot,
    Capability { capabilities: Vec<Capability> },
    ClientBug,
    Closed,
    ContactAdmin,
    CopyUid { uid_validity: u32, uids: Vec<u32> },
    Corruption,
    Expired,
    ExpungeIssued,
    HasChildren,
    InUse,
    Limit,
    NonExistent,
    NoPerm,
    OverQuota,
    Parse,
    PermanentFlags,
    PrivacyRequired,
    ReadOnly,
    ReadWrite,
    ServerBug,
    TryCreate,
    UidNext,
    UidNotSticky,
    UidValidity,
    Unavailable,
    UnknownCte,

    // CONDSTORE
    Modified { ids: Vec<u32> },

    // ObjectID
    MailboxId { mailbox_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusResponse {
    pub tag: Option<String>,
    pub code: Option<ResponseCode>,
    pub message: Cow<'static, str>,
    pub rtype: ResponseType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseType {
    Ok,
    No,
    Bad,
    PreAuth,
    Bye,
}

impl StatusResponse {
    pub fn bad(message: impl Into<Cow<'static, str>>) -> Self {
        StatusResponse {
            tag: None,
            code: None,
            message: message.into(),
            rtype: ResponseType::Bad,
        }
    }

    pub fn parse_error(message: impl Into<Cow<'static, str>>) -> Self {
        StatusResponse {
            tag: None,
            code: ResponseCode::Parse.into(),
            message: message.into(),
            rtype: ResponseType::Bad,
        }
    }

    pub fn database_failure() -> Self {
        StatusResponse::no("Database failure.").with_code(ResponseCode::ContactAdmin)
    }

    pub fn completed(command: Command) -> Self {
        StatusResponse::ok(format!("{} completed", command))
    }

    pub fn with_code(mut self, code: ResponseCode) -> Self {
        self.code = Some(code);
        self
    }

    pub fn with_tag(mut self, tag: String) -> Self {
        self.tag = Some(tag);
        self
    }

    pub fn no(message: impl Into<Cow<'static, str>>) -> Self {
        StatusResponse {
            tag: None,
            code: None,
            message: message.into(),
            rtype: ResponseType::No,
        }
    }

    pub fn ok(message: impl Into<Cow<'static, str>>) -> Self {
        StatusResponse {
            tag: None,
            code: None,
            message: message.into(),
            rtype: ResponseType::Ok,
        }
    }

    pub fn bye(message: impl Into<Cow<'static, str>>) -> Self {
        StatusResponse {
            tag: None,
            code: None,
            message: message.into(),
            rtype: ResponseType::Bye,
        }
    }
}

pub trait IntoStatusResponse {
    fn into_status_response(self) -> StatusResponse;
}

impl IntoStatusResponse for jmap_client::Error {
    fn into_status_response(self) -> StatusResponse {
        let (code, message) = match self {
            jmap_client::Error::Transport(_) => (
                ResponseCode::ContactAdmin,
                "Could not connect to JMAP server.".to_string(),
            ),
            jmap_client::Error::Parse(_) => (
                ResponseCode::ContactAdmin,
                "Failed to parse JMAP server response.".to_string(),
            ),
            jmap_client::Error::Internal(_) => {
                (ResponseCode::ContactAdmin, "Internal Error.".to_string())
            }
            jmap_client::Error::Problem(err) => match err.error() {
                ProblemType::JMAP(err_) => match err_ {
                    JMAPError::UnknownCapability => (
                        ResponseCode::ContactAdmin,
                        "JMAP capability unknown.".to_string(),
                    ),
                    JMAPError::NotJSON => (
                        ResponseCode::ContactAdmin,
                        "JMAP server failed to parse JSON request.".to_string(),
                    ),
                    JMAPError::NotRequest => (
                        ResponseCode::ContactAdmin,
                        "JMAP server could not process the request.".to_string(),
                    ),
                    JMAPError::Limit => (
                        ResponseCode::Limit,
                        match err.limit().unwrap_or("other") {
                            "maxSizeRequest" => "Request size exceeds maximum allowed.",
                            "maxCallsInRequest" => "Too many method calls in the same request.",
                            "maxConcurrentRequests" => "Too many concurrent requests.",
                            _ => "Server limit exceeded.",
                        }
                        .to_string(),
                    ),
                },
                ProblemType::Other(_) => match err.status().unwrap_or(0) {
                    403 => (
                        ResponseCode::NoPerm,
                        "You do not have enough permissions to perform this action.".to_string(),
                    ),
                    429 => (
                        ResponseCode::Limit,
                        "Too many requests, please try again later.".to_string(),
                    ),
                    _ => (
                        ResponseCode::ContactAdmin,
                        format!("Server error, {}", err.detail().unwrap_or("unknown.")),
                    ),
                },
            },
            jmap_client::Error::Server(err) => {
                (ResponseCode::ContactAdmin, format!("Server error, {}", err))
            }
            jmap_client::Error::Method(err) => match err.error() {
                MethodErrorType::ServerUnavailable => (
                    ResponseCode::ContactAdmin,
                    "Server unavailable.".to_string(),
                ),
                MethodErrorType::ServerFail => {
                    (ResponseCode::ContactAdmin, "Server failed.".to_string())
                }
                MethodErrorType::ServerPartialFail => (
                    ResponseCode::ContactAdmin,
                    "Partial server failure.".to_string(),
                ),
                MethodErrorType::UnknownMethod => (
                    ResponseCode::ContactAdmin,
                    "Unknown JMAP Method.".to_string(),
                ),
                MethodErrorType::InvalidArguments => {
                    (ResponseCode::ContactAdmin, "Invalid arguments.".to_string())
                }
                MethodErrorType::InvalidResultReference => (
                    ResponseCode::ContactAdmin,
                    "Invalid result reference.".to_string(),
                ),
                MethodErrorType::Forbidden => {
                    (ResponseCode::NoPerm, "Access forbidden.".to_string())
                }
                MethodErrorType::AccountNotFound => {
                    (ResponseCode::NonExistent, "Account not found.".to_string())
                }
                MethodErrorType::AccountNotSupportedByMethod => (
                    ResponseCode::NoPerm,
                    "Action not supported on this account.".to_string(),
                ),
                MethodErrorType::AccountReadOnly => {
                    (ResponseCode::NoPerm, "Account is read only.".to_string())
                }
                MethodErrorType::RequestTooLarge => {
                    (ResponseCode::Limit, "Request is too large.".to_string())
                }
                MethodErrorType::CannotCalculateChanges => (
                    ResponseCode::Cannot,
                    "Cannot calculate changes.".to_string(),
                ),
                MethodErrorType::StateMismatch => {
                    (ResponseCode::ClientBug, "State mismatch.".to_string())
                }
                MethodErrorType::AlreadyExists => (ResponseCode::AlreadyExists, ".".to_string()),
                MethodErrorType::FromAccountNotFound => (
                    ResponseCode::NonExistent,
                    "Source account not found.".to_string(),
                ),
                MethodErrorType::FromAccountNotSupportedByMethod => (
                    ResponseCode::Cannot,
                    "Action not supported on source account.".to_string(),
                ),
                MethodErrorType::AnchorNotFound => {
                    (ResponseCode::ContactAdmin, "Anchor not found.".to_string())
                }
                MethodErrorType::UnsupportedSort => (
                    ResponseCode::Cannot,
                    "Sort criteria not supported by the server.".to_string(),
                ),
                MethodErrorType::UnsupportedFilter => (
                    ResponseCode::Cannot,
                    "Filter not supported by the server.".to_string(),
                ),
                MethodErrorType::TooManyChanges => {
                    (ResponseCode::Limit, "Too many changes.".to_string())
                }
            },
            jmap_client::Error::Set(err) => match err.error() {
                SetErrorType::Forbidden => (
                    ResponseCode::NoPerm,
                    "You don't have enough permissions.".to_string(),
                ),
                SetErrorType::OverQuota => (
                    ResponseCode::OverQuota,
                    "You have exceeded your quota.".to_string(),
                ),
                SetErrorType::TooLarge => {
                    (ResponseCode::Limit, "Request is too large.".to_string())
                }
                SetErrorType::RateLimit => (
                    ResponseCode::Limit,
                    "Too many requests, please try again later.".to_string(),
                ),
                SetErrorType::NotFound => (ResponseCode::NonExistent, "Not found.".to_string()),
                SetErrorType::InvalidPatch => (
                    ResponseCode::Cannot,
                    "Operation not supported by the server.".to_string(),
                ),
                SetErrorType::WillDestroy => {
                    (ResponseCode::Cannot, "Item will be destroyed.".to_string())
                }
                SetErrorType::InvalidProperties => {
                    (ResponseCode::Cannot, "Invalid properties.".to_string())
                }
                SetErrorType::Singleton => (
                    ResponseCode::Cannot,
                    "Failed operation on singleton.".to_string(),
                ),
                SetErrorType::MailboxHasChild => (
                    ResponseCode::Cannot,
                    "Mailbox has children and cannot be deleted.".to_string(),
                ),
                SetErrorType::MailboxHasEmail => (
                    ResponseCode::Cannot,
                    "Mailbox has messages and cannot be deleted.".to_string(),
                ),
                SetErrorType::BlobNotFound => (
                    ResponseCode::NonExistent,
                    "One or more message parts are not available for retrieval.".to_string(),
                ),
                SetErrorType::TooManyKeywords => {
                    (ResponseCode::Limit, "Too many keywords.".to_string())
                }
                SetErrorType::TooManyMailboxes => {
                    (ResponseCode::Limit, "Too many mailboxes.".to_string())
                }
                SetErrorType::ForbiddenFrom => (
                    ResponseCode::Cannot,
                    "From address is not allowed.".to_string(),
                ),
                SetErrorType::InvalidEmail => {
                    (ResponseCode::Cannot, "Invalid e-mail address.".to_string())
                }
                SetErrorType::TooManyRecipients => {
                    (ResponseCode::Limit, "Too many recipients.".to_string())
                }
                SetErrorType::NoRecipients => {
                    (ResponseCode::Cannot, "No recipients speficied.".to_string())
                }
                SetErrorType::InvalidRecipients => (
                    ResponseCode::Cannot,
                    "One or more recipients are invalid.".to_string(),
                ),
                SetErrorType::ForbiddenMailFrom => {
                    (ResponseCode::Cannot, "Mail from is forbidden.".to_string())
                }
                SetErrorType::ForbiddenToSend => {
                    (ResponseCode::NoPerm, "Sending is not allowed.".to_string())
                }
                SetErrorType::CannotUnsend => (ResponseCode::Cannot, "Cannot unsend.".to_string()),
            },
            jmap_client::Error::WebSocket(_) => (
                ResponseCode::ContactAdmin,
                "WebSockets protocol error.".to_string(),
            ),
        };

        StatusResponse::no(message).with_code(code)
    }
}

pub type Result<T> = std::result::Result<T, StatusResponse>;
