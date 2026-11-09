//! CrabHosts' UI-independent domain model and safe system hosts operations.

mod backup;
mod error;
mod model;
mod parser;
mod paths;
mod privileged;
mod storage;
mod system;
mod system_writer;

pub use backup::{BackupMetadata, BackupStore};
pub use error::{Error, Result};
pub use model::{Document, HostEntry, Profile, validate_hostname};
pub use parser::parse_hosts;
pub use paths::{AppPaths, HOSTS_FILE_ENV};
pub use privileged::{
    HELPER_PROTOCOL_VERSION, HelperRequest, HelperResponse, HelperStatus, HostsWriteRequest,
    MAX_HOSTS_BYTES, execute_privileged_write,
};
pub use storage::{Storage, atomic_write, sha256_hex};
pub use system::{ApplyReport, SystemPlan, apply_plan, plan_profile, plan_restore};
pub use system_writer::write_system_hosts;
