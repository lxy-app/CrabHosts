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
        }
    }

    pub fn is_permission_denied(&self) -> bool {
        matches!(self, Self::Io(error) if error.kind() == std::io::ErrorKind::PermissionDenied)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
