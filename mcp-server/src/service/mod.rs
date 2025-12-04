// Service Management Module
//
// Cross-platform service/daemon support for IronBase MCP Server.
// Provides install/uninstall/start/stop functionality for:
// - Windows: Windows Service (via sc.exe)
// - Linux: systemd daemon
// - macOS: launchd agent

#[cfg(windows)]
pub mod windows;

#[cfg(unix)]
pub mod systemd;

#[cfg(target_os = "macos")]
pub mod launchd;

use std::error::Error;

/// Service command result
pub type ServiceResult<T> = Result<T, Box<dyn Error>>;

/// Service name constant
pub const SERVICE_NAME: &str = "IronBaseService";
pub const SERVICE_DISPLAY_NAME: &str = "IronBase MCP Server";
pub const SERVICE_DESCRIPTION: &str = "IronBase NoSQL database MCP/REST API server";

/// Install the service on the current platform
pub fn install() -> ServiceResult<()> {
    #[cfg(windows)]
    {
        windows::install_service()
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        systemd::install_systemd()
    }

    #[cfg(target_os = "macos")]
    {
        launchd::install_launchd()
    }
}

/// Uninstall the service on the current platform
pub fn uninstall() -> ServiceResult<()> {
    #[cfg(windows)]
    {
        windows::uninstall_service()
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        systemd::uninstall_systemd()
    }

    #[cfg(target_os = "macos")]
    {
        launchd::uninstall_launchd()
    }
}

/// Start the service on the current platform
pub fn start() -> ServiceResult<()> {
    #[cfg(windows)]
    {
        windows::start_service()
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        systemd::start_systemd()
    }

    #[cfg(target_os = "macos")]
    {
        launchd::start_launchd()
    }
}

/// Stop the service on the current platform
pub fn stop() -> ServiceResult<()> {
    #[cfg(windows)]
    {
        windows::stop_service()
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        systemd::stop_systemd()
    }

    #[cfg(target_os = "macos")]
    {
        launchd::stop_launchd()
    }
}

/// Check service status
pub fn status() -> ServiceResult<String> {
    #[cfg(windows)]
    {
        windows::status_service()
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        systemd::status_systemd()
    }

    #[cfg(target_os = "macos")]
    {
        launchd::status_launchd()
    }
}

/// Check if running as a service/daemon
#[allow(dead_code)]
pub fn is_running_as_service() -> bool {
    #[cfg(windows)]
    {
        // Windows services don't have a console
        // This is a simple heuristic
        std::env::var("RUNNING_AS_SERVICE").is_ok()
    }

    #[cfg(unix)]
    {
        // Check for systemd socket activation or other indicators
        std::env::var("NOTIFY_SOCKET").is_ok() || std::env::var("INVOCATION_ID").is_ok()
    }
}
