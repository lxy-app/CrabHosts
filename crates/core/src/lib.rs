//! CrabHosts' UI-independent domain model and safe system hosts operations.

mod backup;
mod error;
mod model;
mod parser;
mod paths;
mod storage;
mod system;

pub use backup::{BackupMetadata, BackupStore};
pub use error::{Error, Result};
pub use model::{Document, HostEntry, Profile, validate_hostname};
pub use parser::parse_hosts;
pub use paths::AppPaths;
pub use storage::{Storage, atomic_write, sha256_hex};
pub use system::{ApplyReport, SystemPlan, apply_plan, plan_profile, plan_restore};
