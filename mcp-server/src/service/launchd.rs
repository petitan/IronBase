// macOS launchd Service Module
//
// Implements launchd plist management for macOS.

use super::{ServiceResult, SERVICE_DESCRIPTION};
use std::fs;
use std::process::Command;

const LAUNCHD_LABEL: &str = "com.ironbase.server";
const LAUNCHD_PLIST_PATH: &str = "/Library/LaunchDaemons/com.ironbase.server.plist";

/// Generate launchd plist content
fn generate_plist() -> ServiceResult<String> {
    let exe_path = std::env::current_exe()?;

    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>

    <key>Comment</key>
    <string>{description}</string>

    <key>ProgramArguments</key>
    <array>
        <string>{exe_path}</string>
    </array>

    <key>WorkingDirectory</key>
    <string>/usr/local/var/ironbase</string>

    <key>EnvironmentVariables</key>
    <dict>
        <key>IRONBASE_PATH</key>
        <string>/usr/local/var/ironbase/data.mlite</string>
        <key>MCP_CONFIG</key>
        <string>/usr/local/etc/ironbase/config.toml</string>
    </dict>

    <key>StandardOutPath</key>
    <string>/usr/local/var/log/ironbase/stdout.log</string>

    <key>StandardErrorPath</key>
    <string>/usr/local/var/log/ironbase/stderr.log</string>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>

    <key>ThrottleInterval</key>
    <integer>10</integer>

    <key>SoftResourceLimits</key>
    <dict>
        <key>NumberOfFiles</key>
        <integer>65536</integer>
    </dict>
</dict>
</plist>
"#,
        label = LAUNCHD_LABEL,
        description = SERVICE_DESCRIPTION,
        exe_path = exe_path.display()
    );

    Ok(content)
}

/// Install the launchd service
pub fn install_launchd() -> ServiceResult<()> {
    let exe_path = std::env::current_exe()?;

    // Check if running as root
    if !is_root() {
        return Err("This operation requires root privileges. Please run with sudo.");
    }

    // Create directories
    fs::create_dir_all("/usr/local/var/ironbase")?;
    fs::create_dir_all("/usr/local/etc/ironbase")?;
    fs::create_dir_all("/usr/local/var/log/ironbase")?;

    // Create default config if not exists
    let config_path = "/usr/local/etc/ironbase/config.toml";
    if !std::path::Path::new(config_path).exists() {
        let default_config = r#"# IronBase MCP Server Configuration

[server]
host = "0.0.0.0"
port = 8080

[database]
path = "/usr/local/var/ironbase/data.mlite"

[logging]
level = "info"
"#;
        fs::write(config_path, default_config)?;
    }

    // Generate and write plist
    let plist_content = generate_plist()?;
    fs::write(LAUNCHD_PLIST_PATH, plist_content)?;

    // Set correct ownership and permissions
    Command::new("chmod")
        .args(["644", LAUNCHD_PLIST_PATH])
        .output()?;

    Command::new("chown")
        .args(["root:wheel", LAUNCHD_PLIST_PATH])
        .output()?;

    println!(
        "launchd service '{}' installed successfully!",
        LAUNCHD_LABEL
    );
    println!("  Plist: {}", LAUNCHD_PLIST_PATH);
    println!("  Executable: {}", exe_path.display());
    println!("  Config: /usr/local/etc/ironbase/config.toml");
    println!("  Data: /usr/local/var/ironbase/");
    println!("  Logs: /usr/local/var/log/ironbase/");
    println!();
    println!("To load: sudo launchctl load {}", LAUNCHD_PLIST_PATH);
    println!("To start: sudo launchctl start {}", LAUNCHD_LABEL);
    println!("To check: sudo launchctl list | grep ironbase");

    Ok(())
}

/// Uninstall the launchd service
pub fn uninstall_launchd() -> ServiceResult<()> {
    if !is_root() {
        return Err("This operation requires root privileges. Please run with sudo.");
    }

    // Stop and unload service
    let _ = Command::new("launchctl")
        .args(["stop", LAUNCHD_LABEL])
        .output();

    let _ = Command::new("launchctl")
        .args(["unload", LAUNCHD_PLIST_PATH])
        .output();

    // Remove plist file
    if std::path::Path::new(LAUNCHD_PLIST_PATH).exists() {
        fs::remove_file(LAUNCHD_PLIST_PATH)?;
    }

    println!("launchd service '{}' uninstalled.", LAUNCHD_LABEL);
    println!("Note: Configuration and data files were preserved.");
    println!("  Config: /usr/local/etc/ironbase/");
    println!("  Data: /usr/local/var/ironbase/");

    Ok(())
}

/// Start the launchd service
pub fn start_launchd() -> ServiceResult<()> {
    // First ensure it's loaded
    let _ = Command::new("launchctl")
        .args(["load", LAUNCHD_PLIST_PATH])
        .output();

    // Then start it
    let output = Command::new("launchctl")
        .args(["start", LAUNCHD_LABEL])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            return Err(format!("Failed to start service: {}", stderr).into());
        }
    }

    println!("Service '{}' started.", LAUNCHD_LABEL);
    Ok(())
}

/// Stop the launchd service
pub fn stop_launchd() -> ServiceResult<()> {
    let output = Command::new("launchctl")
        .args(["stop", LAUNCHD_LABEL])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            return Err(format!("Failed to stop service: {}", stderr).into());
        }
    }

    println!("Service '{}' stopped.", LAUNCHD_LABEL);
    Ok(())
}

/// Get service status
pub fn status_launchd() -> ServiceResult<String> {
    let output = Command::new("launchctl").args(["list"]).output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Check if service is in launchctl list
    for line in stdout.lines() {
        if line.contains(LAUNCHD_LABEL) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let pid = parts[0];
                let status = parts[1];

                if pid == "-" {
                    return Ok("Stopped".to_string());
                } else if status == "0" || pid.parse::<i32>().is_ok() {
                    return Ok(format!("Running (PID: {})", pid));
                } else {
                    return Ok(format!("Failed (exit code: {})", status));
                }
            }
        }
    }

    // Check if plist exists
    if std::path::Path::new(LAUNCHD_PLIST_PATH).exists() {
        Ok("Not loaded".to_string())
    } else {
        Ok("Not installed".to_string())
    }
}

/// Check if running as root
fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_plist() {
        let content = generate_plist().unwrap();
        assert!(content.contains("<plist version=\"1.0\">"));
        assert!(content.contains("<key>Label</key>"));
        assert!(content.contains(LAUNCHD_LABEL));
        assert!(content.contains("<key>RunAtLoad</key>"));
        assert!(content.contains("<key>KeepAlive</key>"));
    }
}
