use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error(
        "confirmation required; repeat with --yes after explicit approval (use --dry-run first for system changes)"
    )]
    ConfirmationRequired,
    #[error(
        "the planned content changed or is stale; create a new plan and request approval again"
    )]
    Conflict,
    #[error("unsafe managed hosts block: {0}")]
    UnsafeBlock(String),
    #[error("privileged helper is unavailable: {0}")]
    HelperUnavailable(String),
    #[error("privileged helper protocol error: {0}")]
    HelperProtocol(String),
    #[error("privileged helper rejected the write: {0}")]
    HelperPermission(String),
    #[error("privileged helper failed to write hosts: {0}")]
    HelperWrite(String),
}

impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                "permission_denied"
            }
            Self::Io(_) => "io_error",
            Self::Json(_) => "invalid_json",
            Self::Invalid(_) => "invalid_input",
            Self::NotFound(_) => "not_found",
            Self::AlreadyExists(_) => "already_exists",
            Self::ConfirmationRequired => "confirmation_required",
            Self::Conflict => "conflict",
            Self::UnsafeBlock(_) => "unsafe_managed_block",
            Self::HelperUnavailable(_) => "helper_unavailable",
            Self::HelperProtocol(_) => "helper_protocol",
            Self::HelperPermission(_) => "permission_denied",
            Self::HelperWrite(_) => "helper_write_failed",
        }
    }

    pub fn is_permission_denied(&self) -> bool {
        matches!(self, Self::Io(error) if error.kind() == std::io::ErrorKind::PermissionDenied)
            || matches!(self, Self::HelperPermission(_))
    }
}

pub type Result<T> = std::result::Result<T, Error>;
