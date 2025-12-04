// Linux systemd Service Module
//
// Implements systemd service management for Linux.

use super::{ServiceResult, SERVICE_DESCRIPTION};
use std::fs;
use std::process::Command;

const SYSTEMD_SERVICE_NAME: &str = "ironbase";
const SYSTEMD_SERVICE_PATH: &str = "/etc/systemd/system/ironbase.service";

/// Generate systemd service unit content
fn generate_service_unit() -> ServiceResult<String> {
    let exe_path = std::env::current_exe()?;

    let content = format!(
        r#"[Unit]
Description={description}
After=network.target
Documentation=https://github.com/petitan/ironbase

[Service]
Type=simple
User=ironbase
Group=ironbase
WorkingDirectory=/var/lib/ironbase
ExecStart={exe_path}
Restart=on-failure
RestartSec=10
StandardOutput=journal
StandardError=journal

# Environment
Environment=IRONBASE_PATH=/var/lib/ironbase/data.mlite
Environment=MCP_CONFIG=/etc/ironbase/config.toml

# Security hardening
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true

# Capability restrictions
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
AmbientCapabilities=CAP_NET_BIND_SERVICE

# File access
ReadWritePaths=/var/lib/ironbase /var/log/ironbase
ReadOnlyPaths=/etc/ironbase

# Network
RestrictAddressFamilies=AF_INET AF_INET6

[Install]
WantedBy=multi-user.target
"#,
        description = SERVICE_DESCRIPTION,
        exe_path = exe_path.display()
    );

    Ok(content)
}

/// Install the systemd service
pub fn install_systemd() -> ServiceResult<()> {
    let exe_path = std::env::current_exe()?;

    // Check if running as root
    if !is_root() {
        return Err("This operation requires root privileges. Please run with sudo.".into());
    }

    // Create ironbase user if it doesn't exist
    let _ = Command::new("useradd")
        .args(["-r", "-s", "/bin/false", "-d", "/var/lib/ironbase", "ironbase"])
        .output();

    // Create directories
    fs::create_dir_all("/var/lib/ironbase")?;
    fs::create_dir_all("/etc/ironbase")?;
    fs::create_dir_all("/var/log/ironbase")?;

    // Set ownership
    Command::new("chown")
        .args(["-R", "ironbase:ironbase", "/var/lib/ironbase"])
        .output()?;

    Command::new("chown")
        .args(["-R", "ironbase:ironbase", "/var/log/ironbase"])
        .output()?;

    // Create default config if not exists
    let config_path = "/etc/ironbase/config.toml";
    if !std::path::Path::new(config_path).exists() {
        let default_config = r#"# IronBase MCP Server Configuration

[server]
host = "0.0.0.0"
port = 8080

[database]
path = "/var/lib/ironbase/data.mlite"

[logging]
level = "info"
"#;
        fs::write(config_path, default_config)?;
    }

    // Generate and write service unit
    let service_content = generate_service_unit()?;
    fs::write(SYSTEMD_SERVICE_PATH, service_content)?;

    // Set permissions on service file
    Command::new("chmod")
        .args(["644", SYSTEMD_SERVICE_PATH])
        .output()?;

    // Reload systemd
    Command::new("systemctl")
        .args(["daemon-reload"])
        .output()?;

    // Enable service
    Command::new("systemctl")
        .args(["enable", SYSTEMD_SERVICE_NAME])
        .output()?;

    println!("systemd service '{}' installed successfully!", SYSTEMD_SERVICE_NAME);
    println!("  Service file: {}", SYSTEMD_SERVICE_PATH);
    println!("  Executable: {}", exe_path.display());
    println!("  Config: /etc/ironbase/config.toml");
    println!("  Data: /var/lib/ironbase/");
    println!();
    println!("To start: sudo systemctl start {}", SYSTEMD_SERVICE_NAME);
    println!("To check status: sudo systemctl status {}", SYSTEMD_SERVICE_NAME);
    println!("To view logs: sudo journalctl -u {} -f", SYSTEMD_SERVICE_NAME);

    Ok(())
}

/// Uninstall the systemd service
pub fn uninstall_systemd() -> ServiceResult<()> {
    if !is_root() {
        return Err("This operation requires root privileges. Please run with sudo.".into());
    }

    // Stop service
    let _ = Command::new("systemctl")
        .args(["stop", SYSTEMD_SERVICE_NAME])
        .output();

    // Disable service
    let _ = Command::new("systemctl")
        .args(["disable", SYSTEMD_SERVICE_NAME])
        .output();

    // Remove service file
    if std::path::Path::new(SYSTEMD_SERVICE_PATH).exists() {
        fs::remove_file(SYSTEMD_SERVICE_PATH)?;
    }

    // Reload systemd
    Command::new("systemctl")
        .args(["daemon-reload"])
        .output()?;

    println!("systemd service '{}' uninstalled.", SYSTEMD_SERVICE_NAME);
    println!("Note: Configuration and data files were preserved.");
    println!("  Config: /etc/ironbase/");
    println!("  Data: /var/lib/ironbase/");

    Ok(())
}

/// Start the systemd service
pub fn start_systemd() -> ServiceResult<()> {
    let output = Command::new("systemctl")
        .args(["start", SYSTEMD_SERVICE_NAME])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to start service: {}", stderr).into());
    }

    println!("Service '{}' started.", SYSTEMD_SERVICE_NAME);
    Ok(())
}

/// Stop the systemd service
pub fn stop_systemd() -> ServiceResult<()> {
    let output = Command::new("systemctl")
        .args(["stop", SYSTEMD_SERVICE_NAME])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to stop service: {}", stderr).into());
    }

    println!("Service '{}' stopped.", SYSTEMD_SERVICE_NAME);
    Ok(())
}

/// Get service status
pub fn status_systemd() -> ServiceResult<String> {
    let output = Command::new("systemctl")
        .args(["is-active", SYSTEMD_SERVICE_NAME])
        .output()?;

    let status = String::from_utf8_lossy(&output.stdout).trim().to_string();

    match status.as_str() {
        "active" => Ok("Running".to_string()),
        "inactive" => Ok("Stopped".to_string()),
        "failed" => Ok("Failed".to_string()),
        _ => {
            // Check if installed
            let exists = std::path::Path::new(SYSTEMD_SERVICE_PATH).exists();
            if exists {
                Ok(format!("Unknown ({})", status))
            } else {
                Ok("Not installed".to_string())
            }
        }
    }
}

/// Check if running under systemd
#[allow(dead_code)]
pub fn is_systemd() -> bool {
    std::env::var("NOTIFY_SOCKET").is_ok() || std::env::var("INVOCATION_ID").is_ok()
}

/// Check if running as root
fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_service_unit() {
        let content = generate_service_unit().unwrap();
        assert!(content.contains("[Unit]"));
        assert!(content.contains("[Service]"));
        assert!(content.contains("[Install]"));
        assert!(content.contains("Type=simple"));
        assert!(content.contains("Restart=on-failure"));
    }
}
