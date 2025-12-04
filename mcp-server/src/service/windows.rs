// Windows Service Module
//
// Implements Windows Service management using sc.exe commands.
// For full Windows Service integration, the windows-service crate is available.

use super::{ServiceResult, SERVICE_DESCRIPTION, SERVICE_DISPLAY_NAME, SERVICE_NAME};
use std::process::Command;

/// Install the Windows Service
pub fn install_service() -> ServiceResult<()> {
    let exe_path = std::env::current_exe()?;

    // Create service using sc.exe
    let output = Command::new("sc")
        .args([
            "create",
            SERVICE_NAME,
            &format!("binPath= \"{}\"", exe_path.display()),
            "start= auto",
            &format!("DisplayName= \"{}\"", SERVICE_DISPLAY_NAME),
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("already exists") {
            println!("Service already installed, updating...");
            // Update existing service
            Command::new("sc")
                .args([
                    "config",
                    SERVICE_NAME,
                    &format!("binPath= \"{}\"", exe_path.display()),
                ])
                .output()?;
        } else {
            return Err(format!("Failed to create service: {}", stderr).into());
        }
    }

    // Set service description
    Command::new("sc")
        .args(["description", SERVICE_NAME, SERVICE_DESCRIPTION])
        .output()?;

    // Set failure recovery: restart after 60 seconds on failure
    Command::new("sc")
        .args([
            "failure",
            SERVICE_NAME,
            "reset= 86400",
            "actions= restart/60000/restart/60000/restart/60000",
        ])
        .output()?;

    println!("Windows Service '{}' installed successfully!", SERVICE_NAME);
    println!("  Display Name: {}", SERVICE_DISPLAY_NAME);
    println!("  Executable: {}", exe_path.display());
    println!();
    println!("To start: sc start {}", SERVICE_NAME);
    println!("Or use: {} start", exe_path.display());

    Ok(())
}

/// Uninstall the Windows Service
pub fn uninstall_service() -> ServiceResult<()> {
    // Stop service first (ignore errors if not running)
    let _ = Command::new("sc").args(["stop", SERVICE_NAME]).output();

    // Wait a moment for service to stop
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Delete service
    let output = Command::new("sc")
        .args(["delete", SERVICE_NAME])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("does not exist") {
            return Err(format!("Failed to delete service: {}", stderr).into());
        }
    }

    println!("Windows Service '{}' uninstalled successfully!", SERVICE_NAME);

    Ok(())
}

/// Start the Windows Service
pub fn start_service() -> ServiceResult<()> {
    let output = Command::new("sc")
        .args(["start", SERVICE_NAME])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("already running") {
            println!("Service is already running.");
            return Ok(());
        }
        return Err(format!("Failed to start service: {}", stderr).into());
    }

    println!("Service '{}' started.", SERVICE_NAME);
    Ok(())
}

/// Stop the Windows Service
pub fn stop_service() -> ServiceResult<()> {
    let output = Command::new("sc")
        .args(["stop", SERVICE_NAME])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("not started") || stderr.contains("not been started") {
            println!("Service is not running.");
            return Ok(());
        }
        return Err(format!("Failed to stop service: {}", stderr).into());
    }

    println!("Service '{}' stopped.", SERVICE_NAME);
    Ok(())
}

/// Get service status
pub fn status_service() -> ServiceResult<String> {
    let output = Command::new("sc")
        .args(["query", SERVICE_NAME])
        .output()?;

    if !output.status.success() {
        return Ok("Not installed".to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse state from sc query output
    if stdout.contains("RUNNING") {
        Ok("Running".to_string())
    } else if stdout.contains("STOPPED") {
        Ok("Stopped".to_string())
    } else if stdout.contains("PENDING") {
        Ok("Pending".to_string())
    } else {
        Ok("Unknown".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_name_constants() {
        assert!(!SERVICE_NAME.is_empty());
        assert!(!SERVICE_DISPLAY_NAME.is_empty());
        assert!(!SERVICE_DESCRIPTION.is_empty());
    }
}
