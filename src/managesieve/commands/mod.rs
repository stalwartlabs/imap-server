/*
 * Copyright (c) 2020-2022, Stalwart Labs Ltd.
 *
 * This file is part of the Stalwart IMAP Server.
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of
 * the License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 * in the LICENSE file at the top-level directory of this distribution.
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/>.
 *
 * You can be released from the requirements of the AGPLv3 license by
 * purchasing a commercial license. Please contact licensing@stalw.art
 * for more details.
*/

use std::borrow::Cow;

use jmap_client::core::{
    error::{JMAPError, MethodErrorType, ProblemType},
    set::SetErrorType,
};

use super::{ResponseCode, StatusResponse};

pub mod authenticate;
pub mod capability;
pub mod checkscript;
pub mod deletescript;
pub mod getscript;
pub mod havespace;
pub mod listscripts;
pub mod logout;
pub mod noop;
pub mod putscript;
pub mod renamescript;
pub mod setactive;
pub mod starttls;

pub trait IntoStatusResponse {
    fn into_status_response(self) -> StatusResponse;
}

impl IntoStatusResponse for jmap_client::Error {
    fn into_status_response(self) -> StatusResponse {
        let (code, message): (Option<ResponseCode>, Cow<'static, str>) = match self {
            jmap_client::Error::Transport(_) => (None, "Could not connect to JMAP server.".into()),
            jmap_client::Error::Parse(_) => (None, "Failed to parse JMAP server response.".into()),
            jmap_client::Error::Internal(_) => (None, "Internal Error.".into()),
            jmap_client::Error::Problem(err) => match err.error() {
                ProblemType::JMAP(err_) => match err_ {
                    JMAPError::UnknownCapability => (None, "JMAP capability unknown.".into()),
                    JMAPError::NotJSON => {
                        (None, "JMAP server failed to parse JSON request.".into())
                    }
                    JMAPError::NotRequest => {
                        (None, "JMAP server could not process the request.".into())
                    }
                    JMAPError::Limit => (
                        ResponseCode::Quota.into(),
                        match err.limit().unwrap_or("other") {
                            "maxSizeRequest" => "Request size exceeds maximum allowed.",
                            "maxCallsInRequest" => "Too many method calls in the same request.",
                            "maxConcurrentRequests" => "Too many concurrent requests.",
                            _ => "Server limit exceeded.",
                        }
                        .into(),
                    ),
                },
                ProblemType::Other(_) => match err.status().unwrap_or(0) {
                    403 => (
                        None,
                        "You do not have enough permissions to perform this action.".into(),
                    ),
                    429 => (
                        ResponseCode::Quota.into(),
                        "Too many requests, please try again later.".into(),
                    ),
                    _ => (
                        None,
                        format!("Server error, {}", err.detail().unwrap_or("unknown.")).into(),
                    ),
                },
            },
            jmap_client::Error::Server(err) => (
                ResponseCode::TryLater.into(),
                format!("Server error, {}", err).into(),
            ),
            jmap_client::Error::Method(err) => match err.error() {
                MethodErrorType::ServerUnavailable => {
                    (ResponseCode::TryLater.into(), "Server unavailable.".into())
                }
                MethodErrorType::ServerFail => {
                    (ResponseCode::TryLater.into(), "Server failed.".into())
                }
                MethodErrorType::ServerPartialFail => (
                    ResponseCode::TryLater.into(),
                    "Partial server failure.".into(),
                ),
                MethodErrorType::UnknownMethod => (None, "Unknown JMAP Method.".into()),
                MethodErrorType::InvalidArguments => (None, "Invalid arguments.".into()),
                MethodErrorType::InvalidResultReference => {
                    (None, "Invalid result reference.".into())
                }
                MethodErrorType::Forbidden => (None, "Access forbidden.".into()),
                MethodErrorType::AccountNotFound => (
                    ResponseCode::NonExistent.into(),
                    "Account not found.".into(),
                ),
                MethodErrorType::AccountNotSupportedByMethod => {
                    (None, "Action not supported on this account.".into())
                }
                MethodErrorType::AccountReadOnly => (None, "Account is read only.".into()),
                MethodErrorType::RequestTooLarge => {
                    (ResponseCode::Quota.into(), "Request is too large.".into())
                }
                MethodErrorType::CannotCalculateChanges => {
                    (None, "Cannot calculate changes.".into())
                }
                MethodErrorType::StateMismatch => (None, "State mismatch.".into()),
                MethodErrorType::AlreadyExists => (ResponseCode::AlreadyExists.into(), ".".into()),
                MethodErrorType::FromAccountNotFound => (
                    ResponseCode::NonExistent.into(),
                    "Source account not found.".into(),
                ),
                MethodErrorType::FromAccountNotSupportedByMethod => {
                    (None, "Action not supported on source account.".into())
                }
                MethodErrorType::AnchorNotFound => (None, "Anchor not found.".into()),
                MethodErrorType::UnsupportedSort => {
                    (None, "Sort criteria not supported by the server.".into())
                }
                MethodErrorType::UnsupportedFilter => {
                    (None, "Filter not supported by the server.".into())
                }
                MethodErrorType::TooManyChanges => {
                    (ResponseCode::Quota.into(), "Too many changes.".into())
                }
            },
            jmap_client::Error::Set(err) => match err.error() {
                SetErrorType::Forbidden => (None, "You don't have enough permissions.".into()),
                SetErrorType::OverQuota => (
                    ResponseCode::Quota.into(),
                    "You have exceeded your quota.".into(),
                ),
                SetErrorType::TooLarge => {
                    (ResponseCode::Quota.into(), "Request is too large.".into())
                }
                SetErrorType::RateLimit => (
                    ResponseCode::Quota.into(),
                    "Too many requests, please try again later.".into(),
                ),
                SetErrorType::NotFound => (ResponseCode::NonExistent.into(), "Not found.".into()),
                SetErrorType::InvalidPatch => {
                    (None, "Operation not supported by the server.".into())
                }
                SetErrorType::WillDestroy => (None, "Item will be destroyed.".into()),
                SetErrorType::InvalidProperties => (
                    None,
                    err.description()
                        .map(|d| d.to_string().into())
                        .unwrap_or_else(|| "Invalid properties.".into()),
                ),
                SetErrorType::Singleton => (None, "Failed operation on singleton.".into()),
                SetErrorType::MailboxHasChild => {
                    (None, "Mailbox has children and cannot be deleted.".into())
                }
                SetErrorType::MailboxHasEmail => {
                    (None, "Mailbox has messages and cannot be deleted.".into())
                }
                SetErrorType::BlobNotFound => (
                    ResponseCode::NonExistent.into(),
                    "One or more message parts are not available for retrieval.".into(),
                ),
                SetErrorType::TooManyKeywords => {
                    (ResponseCode::Quota.into(), "Too many keywords.".into())
                }
                SetErrorType::TooManyMailboxes => {
                    (ResponseCode::Quota.into(), "Too many mailboxes.".into())
                }
                SetErrorType::ForbiddenFrom => (None, "From address is not allowed.".into()),
                SetErrorType::InvalidEmail => (None, "Invalid e-mail address.".into()),
                SetErrorType::TooManyRecipients => {
                    (ResponseCode::Quota.into(), "Too many recipients.".into())
                }
                SetErrorType::NoRecipients => (None, "No recipients speficied.".into()),
                SetErrorType::InvalidRecipients => {
                    (None, "One or more recipients are invalid.".into())
                }
                SetErrorType::ForbiddenMailFrom => (None, "Mail from is forbidden.".into()),
                SetErrorType::ForbiddenToSend => (None, "Sending is not allowed.".into()),
                SetErrorType::CannotUnsend => (None, "Cannot unsend.".into()),
                SetErrorType::AlreadyExists => (
                    ResponseCode::AlreadyExists.into(),
                    "The referenced script name already exists.".into(),
                ),
                SetErrorType::InvalidScript => (
                    None,
                    err.description()
                        .map(|d| d.to_string().into())
                        .unwrap_or_else(|| "Invalid script".into()),
                ),
                SetErrorType::ScriptIsActive => (
                    ResponseCode::Active.into(),
                    "You may not delete an active script.".into(),
                ),
            },
            jmap_client::Error::WebSocket(_) => (None, "WebSockets protocol error.".into()),
        };

        if let Some(code) = code {
            StatusResponse::no(message).with_code(code)
        } else {
            StatusResponse::no(message)
        }
    }
}
