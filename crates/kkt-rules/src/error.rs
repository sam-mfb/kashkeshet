use crate::Permission;
use kkt_store::StoreError;
use serde::Serialize;

/// The fixed error codes agents see.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    Unauthenticated,
    NotFound,
    NotMember,
    BodyTooLong,
    ContainsSecret,
    RateLimited,
    /// A server-wide limit, such as the most agents, has been reached.
    LimitReached,
    InvalidArgument,
    Internal,
    /// Only the channel's owner may do this.
    NotOwner,
    /// Another avatar already has the handle proposed at `register`.
    HandleTaken,
    /// The lane an agent asked to take, at `register`, `reclaim` or
    /// `take_lane`, does not exist or is retired.
    LaneUnavailable,
    /// Only the operator, the human or a named admin agent, may do this.
    OperatorOnly,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Code::Unauthenticated => "unauthenticated",
            Code::NotFound => "not_found",
            Code::NotMember => "not_member",
            Code::BodyTooLong => "body_too_long",
            Code::ContainsSecret => "contains_secret",
            Code::RateLimited => "rate_limited",
            Code::LimitReached => "limit_reached",
            Code::InvalidArgument => "invalid_argument",
            Code::Internal => "internal",
            Code::NotOwner => "not_owner",
            Code::HandleTaken => "handle_taken",
            Code::LaneUnavailable => "lane_unavailable",
            Code::OperatorOnly => "operator_only",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{}: {message}", code.as_str())]
pub struct RuleError {
    pub code: Code,
    pub message: String,
    /// For a refusal a caller may word for its own surface, which one it
    /// is. `message` words it plainly; an edge that knows better, such as
    /// the web page or the admin endpoint, may say more. `None` for every
    /// other error.
    pub refusal: Option<Refusal>,
}

/// A refusal an edge may word for its own surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The operator lacks this permission.
    LacksPermission(Permission),
    /// The avatar `handle` owns the channels with these IDs, in name order,
    /// so it cannot be hidden until each has another owner or none.
    OwnsChannels {
        handle: String,
        channels: Vec<String>,
    },
}

impl RuleError {
    pub fn new(code: Code, message: impl Into<String>) -> RuleError {
        RuleError {
            code,
            message: message.into(),
            refusal: None,
        }
    }

    /// `self`, marked as the refusal `refusal`.
    pub fn because(self, refusal: Refusal) -> RuleError {
        RuleError {
            refusal: Some(refusal),
            ..self
        }
    }

    pub fn unauthenticated() -> RuleError {
        RuleError::new(
            Code::Unauthenticated,
            "missing or unknown token. Call `register` (no arguments) to get a handle and token, \
             then pass `token` on every other call.",
        )
    }

    pub fn channel_not_found(id: &str) -> RuleError {
        RuleError::new(Code::NotFound, format!("no channel {id}"))
    }

    pub fn agent_not_found(handle: &str) -> RuleError {
        RuleError::new(Code::NotFound, format!("no agent with the handle {handle}"))
    }

    pub fn lane_not_found(name: &str) -> RuleError {
        RuleError::new(Code::NotFound, format!("no lane named {name}"))
    }

    pub fn message_not_found(id: i64) -> RuleError {
        RuleError::new(Code::NotFound, format!("no message {id}"))
    }

    pub fn not_member(id: &str) -> RuleError {
        RuleError::new(
            Code::NotMember,
            format!("you are not a member of {id}. It is open: call `join_channel` first."),
        )
    }

    pub fn not_owner(id: &str) -> RuleError {
        RuleError::new(
            Code::NotOwner,
            format!("only the owner of {id} may do this. `list_members` shows the owner."),
        )
    }

    /// The refusal for an agent asking for the operator's change: `what`
    /// says what it asked for, such as "save a lane".
    pub fn operator_only(what: &str) -> RuleError {
        RuleError::new(Code::OperatorOnly, format!("only the operator may {what}"))
    }

    pub fn invalid(message: impl Into<String>) -> RuleError {
        RuleError::new(Code::InvalidArgument, message)
    }
}

impl From<StoreError> for RuleError {
    fn from(e: StoreError) -> RuleError {
        RuleError::new(Code::Internal, e.to_string())
    }
}
