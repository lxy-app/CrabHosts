//! CrabHosts' UI-independent domain model and safe system hosts operations.

mod activation;
mod backup;
mod error;
mod model;
mod parser;
mod paths;
mod privileged;
mod storage;
mod system;
mod system_writer;

pub use activation::{ACTIVATION_SCHEMA_VERSION, ActivationState};
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
pub use system::{
    ApplyReport, ConflictMapping, HostConflict, SystemPlan, apply_plan, plan_profile,
    plan_profiles, plan_restore, render_profiles,
};
pub use system_writer::write_system_hosts;
