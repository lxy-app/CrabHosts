use crabhosts_core::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginItemStatus {
    Enabled,
    RequiresApproval,
    NotInstalled,
    Unavailable,
}

impl LoginItemStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::RequiresApproval => "requires-approval",
            Self::NotInstalled => "not-installed",
            Self::Unavailable => "unavailable",
        }
    }

    pub fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled | Self::RequiresApproval)
    }
}

#[cfg(target_os = "macos")]
pub fn status() -> Result<LoginItemStatus> {
    use objc2_service_management::{SMAppService, SMAppServiceStatus};

    // SAFETY: mainAppService and status are readonly queries on the current application.
    let service = unsafe { SMAppService::mainAppService() };
    Ok(match unsafe { service.status() } {
        SMAppServiceStatus::Enabled => LoginItemStatus::Enabled,
        SMAppServiceStatus::RequiresApproval => LoginItemStatus::RequiresApproval,
        SMAppServiceStatus::NotRegistered | SMAppServiceStatus::NotFound => {
            LoginItemStatus::NotInstalled
        }
        _ => LoginItemStatus::Unavailable,
    })
}

#[cfg(not(target_os = "macos"))]
pub fn status() -> Result<LoginItemStatus> {
    Ok(LoginItemStatus::Unavailable)
}

#[cfg(target_os = "macos")]
pub fn set_enabled(enabled: bool) -> Result<()> {
    use objc2_service_management::{SMAppService, SMAppServiceStatus};

    // SAFETY: SMAppService uses the current app bundle; no path, shell command, or credential is
    // accepted from the UI. The call is made only after an explicit preference interaction.
    let service = unsafe { SMAppService::mainAppService() };
    let status = unsafe { service.status() };
    if enabled
        && matches!(
            status,
            SMAppServiceStatus::Enabled | SMAppServiceStatus::RequiresApproval
        )
    {
        return Ok(());
    }
    if !enabled
        && matches!(
            status,
            SMAppServiceStatus::NotRegistered | SMAppServiceStatus::NotFound
        )
    {
        return Ok(());
    }
    let result = if enabled {
        unsafe { service.registerAndReturnError() }
    } else {
        unsafe { service.unregisterAndReturnError() }
    };
    result.map_err(|error| {
        Error::Invalid(format!(
            "macOS could not update the login item: {}",
            error.localizedDescription()
        ))
    })
}

#[cfg(not(target_os = "macos"))]
pub fn set_enabled(enabled: bool) -> Result<()> {
    if enabled {
        Err(Error::Invalid(
            "login-item preferences are currently supported only on macOS".into(),
        ))
    } else {
        Ok(())
    }
}
