use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::PathBuf;

use crabhosts_core::{
    AppPaths, ApplyReport, Error, HELPER_PROTOCOL_VERSION, HelperRequest, HelperResponse,
    HelperStatus, HostsWriteRequest, Result, Storage, execute_privileged_write,
};

const HELPER_IDENTIFIER: &str = "dev.crabhosts.hosts-helper";
const DESKTOP_IDENTIFIER: &str = "dev.crabhosts.desktop";
const CLI_IDENTIFIER: &str = "dev.crabhosts.cli";

type RequestHandler = unsafe extern "C" fn(*const c_char) -> *mut c_char;

unsafe extern "C" {
    fn crabhosts_xpc_send(
        request: *const c_char,
        helper_requirement: *const c_char,
        error: *mut *mut c_char,
    ) -> *mut c_char;
    fn crabhosts_helper_registration_status() -> c_int;
    fn crabhosts_helper_register(error: *mut *mut c_char) -> c_int;
    fn crabhosts_helper_serve(
        client_requirement: *const c_char,
        handler: RequestHandler,
        error: *mut *mut c_char,
    ) -> c_int;
    fn crabhosts_process_is_root() -> bool;
    fn crabhosts_native_string_free(value: *mut c_char);
    fn malloc(size: usize) -> *mut c_void;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HelperRegistrationStatus {
    NotRegistered,
    Enabled,
    RequiresApproval,
    NotFound,
    Unsupported,
}

impl HelperRegistrationStatus {
    fn from_native(value: c_int) -> Self {
        match value {
            0 => Self::NotRegistered,
            1 => Self::Enabled,
            2 => Self::RequiresApproval,
            3 => Self::NotFound,
            _ => Self::Unsupported,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::NotRegistered => "not_registered",
            Self::Enabled => "enabled",
            Self::RequiresApproval => "requires_approval",
            Self::NotFound => "not_found",
            Self::Unsupported => "unsupported",
        }
    }
}

pub fn helper_registration_status() -> Result<HelperRegistrationStatus> {
    let value = unsafe { crabhosts_helper_registration_status() };
    if value < 0 {
        return Err(Error::HelperUnavailable(
            "SMAppService requires a signed macOS 13+ app bundle".into(),
        ));
    }
    Ok(HelperRegistrationStatus::from_native(value))
}

pub fn register_helper() -> Result<HelperRegistrationStatus> {
    let mut error = std::ptr::null_mut();
    let value = unsafe { crabhosts_helper_register(&mut error) };
    if value < 0 {
        return Err(Error::HelperUnavailable(take_native_string(error)));
    }
    Ok(HelperRegistrationStatus::from_native(value))
}

pub fn helper_status() -> Result<HelperStatus> {
    match round_trip(&HelperRequest::Status {
        protocol_version: HELPER_PROTOCOL_VERSION,
    })? {
        HelperResponse::Status(status) if status.protocol_version == HELPER_PROTOCOL_VERSION => {
            Ok(status)
        }
        HelperResponse::Status(_) => Err(Error::HelperProtocol(
            "helper and client use incompatible protocol versions".into(),
        )),
        HelperResponse::Error { code, message } => Err(remote_error(&code, &message)),
        _ => Err(Error::HelperProtocol(
            "helper returned the wrong status response".into(),
        )),
    }
}

pub fn write(request: HostsWriteRequest) -> Result<ApplyReport> {
    match round_trip(&HelperRequest::Write(request))? {
        HelperResponse::Write(report) => Ok(report),
        HelperResponse::Error { code, message } => Err(remote_error(&code, &message)),
        _ => Err(Error::HelperProtocol(
            "helper returned the wrong write response".into(),
        )),
    }
}

fn round_trip(request: &HelperRequest) -> Result<HelperResponse> {
    let request = CString::new(serde_json::to_vec(request)?)
        .map_err(|_| Error::HelperProtocol("request contains a NUL byte".into()))?;
    let requirement = signing_requirement(HELPER_IDENTIFIER)?;
    let mut native_error = std::ptr::null_mut();
    let response =
        unsafe { crabhosts_xpc_send(request.as_ptr(), requirement.as_ptr(), &mut native_error) };
    if response.is_null() {
        return Err(Error::HelperUnavailable(take_native_string(native_error)));
    }
    let response = take_native_string(response);
    serde_json::from_str(&response).map_err(|error| {
        Error::HelperProtocol(format!("helper returned invalid response JSON: {error}"))
    })
}

fn signing_requirement(identifier: &str) -> Result<CString> {
    let team = option_env!("CRABHOSTS_TEAM_ID").ok_or_else(|| {
        Error::HelperUnavailable(
            "this build has no signing team; use scripts/package-macos.sh for a signed build"
                .into(),
        )
    })?;
    let requirement = format!(
        "anchor apple generic and certificate leaf[subject.OU] = \"{team}\" and identifier \"{identifier}\""
    );
    CString::new(requirement)
        .map_err(|_| Error::HelperProtocol("invalid code signing requirement".into()))
}

fn remote_error(code: &str, message: &str) -> Error {
    match code {
        "conflict" => Error::Conflict,
        "permission_denied" => Error::HelperPermission(message.to_owned()),
        "invalid_input" => Error::Invalid(message.to_owned()),
        "unsafe_managed_block" => Error::UnsafeBlock(message.to_owned()),
        "helper_protocol" => Error::HelperProtocol(message.to_owned()),
        _ => Error::HelperWrite(format!("{code}: {message}")),
    }
}

fn take_native_string(value: *mut c_char) -> String {
    if value.is_null() {
        return "unknown native bridge error".into();
    }
    let message = unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned();
    unsafe { crabhosts_native_string_free(value) };
    message
}

/// Runs the root helper. The launchd daemon must call this from its own signed executable.
pub fn serve_helper() -> Result<()> {
    if !unsafe { crabhosts_process_is_root() } {
        return Err(Error::HelperUnavailable(
            "the hosts helper must be launched by launchd as root".into(),
        ));
    }
    let requirement = client_signing_requirement()?;
    let mut error = std::ptr::null_mut();
    let status =
        unsafe { crabhosts_helper_serve(requirement.as_ptr(), handle_native_request, &mut error) };
    if status == 0 {
        Ok(())
    } else {
        Err(Error::HelperUnavailable(take_native_string(error)))
    }
}

fn client_signing_requirement() -> Result<CString> {
    let team = option_env!("CRABHOSTS_TEAM_ID")
        .ok_or_else(|| Error::HelperUnavailable("helper build has no CRABHOSTS_TEAM_ID".into()))?;
    let requirement = format!(
        "anchor apple generic and certificate leaf[subject.OU] = \"{team}\" and (identifier \"{DESKTOP_IDENTIFIER}\" or identifier \"{CLI_IDENTIFIER}\")"
    );
    CString::new(requirement)
        .map_err(|_| Error::HelperProtocol("invalid client signing requirement".into()))
}

unsafe extern "C" fn handle_native_request(request: *const c_char) -> *mut c_char {
    let response = std::panic::catch_unwind(|| {
        if request.is_null() {
            return HelperResponse::Error {
                code: "helper_protocol".into(),
                message: "null request".into(),
            };
        }
        let request = unsafe { CStr::from_ptr(request) }.to_string_lossy();
        dispatch_request(&request)
    })
    .unwrap_or_else(|_| HelperResponse::Error {
        code: "helper_internal".into(),
        message: "helper request handler panicked".into(),
    });
    let json = serde_json::to_vec(&response).unwrap_or_else(|_| {
        br#"{"error":{"code":"helper_internal","message":"response serialization failed"}}"#
            .to_vec()
    });
    unsafe { malloc_c_string(&json) }
}

fn dispatch_request(request: &str) -> HelperResponse {
    let request = match serde_json::from_str::<HelperRequest>(request) {
        Ok(request) => request,
        Err(error) => {
            return HelperResponse::Error {
                code: "helper_protocol".into(),
                message: format!("invalid request JSON: {error}"),
            };
        }
    };
    match request {
        HelperRequest::Status { protocol_version } => {
            if protocol_version != HELPER_PROTOCOL_VERSION {
                return HelperResponse::Error {
                    code: "helper_protocol".into(),
                    message: "unsupported protocol version".into(),
                };
            }
            HelperResponse::Status(HelperStatus {
                protocol_version: HELPER_PROTOCOL_VERSION,
                helper_version: env!("CARGO_PKG_VERSION").into(),
            })
        }
        HelperRequest::Write(request) => {
            // Deliberately do not call Storage::from_env: a root helper must never honor the
            // invoking user's CRABHOSTS_HOME or CRABHOSTS_HOSTS_FILE.
            let storage = Storage::new(AppPaths::new(
                PathBuf::from("/var/db/CrabHosts"),
                PathBuf::from("/etc/hosts"),
            ));
            match execute_privileged_write(&storage, &request) {
                Ok(report) => HelperResponse::Write(report),
                Err(error) => HelperResponse::error(&error),
            }
        }
    }
}

unsafe fn malloc_c_string(bytes: &[u8]) -> *mut c_char {
    let output = unsafe { malloc(bytes.len() + 1) }.cast::<u8>();
    if output.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len());
        *output.add(bytes.len()) = 0;
    }
    output.cast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_uses_the_current_protocol() {
        let response = dispatch_request(r#"{"status":{"protocol_version":1}}"#);
        assert!(matches!(response, HelperResponse::Status(_)));
    }

    #[test]
    fn malformed_requests_do_not_reach_the_filesystem() {
        let response = dispatch_request("not json");
        assert!(matches!(
            response,
            HelperResponse::Error { ref code, .. } if code == "helper_protocol"
        ));
    }
}
