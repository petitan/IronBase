// Windows Service Module
//
// Implements proper Windows Service integration using windows-service crate.
// The service responds to Windows Service Control Manager (SCM) commands.

#[cfg(windows)]
use std::ffi::OsString;
#[cfg(windows)]
use std::sync::mpsc;
#[cfg(windows)]
use std::time::Duration;
#[cfg(windows)]
use windows_service::{
    define_windows_service,
    service::{
        ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
        ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};

use super::{ServiceResult, SERVICE_DESCRIPTION, SERVICE_DISPLAY_NAME, SERVICE_NAME};

#[cfg(windows)]
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

/// Install the Windows Service
pub fn install_service() -> ServiceResult<()> {
    #[cfg(windows)]
    {
        let exe_path = std::env::current_exe()?;

        // First check if service already exists
        let manager_read =
            ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;

        if let Ok(_existing) = manager_read.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS)
        {
            println!("Service '{}' already exists.", SERVICE_NAME);
            println!("Use 'uninstall' first if you want to reinstall.");
            return Ok(());
        }

        // Service doesn't exist, create it
        let manager =
            ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE)
                .map_err(|e| format!("Failed to connect to Service Manager: {}", e))?;

        let service_info = ServiceInfo {
            name: OsString::from(SERVICE_NAME),
            display_name: OsString::from(SERVICE_DISPLAY_NAME),
            service_type: SERVICE_TYPE,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: exe_path.clone(),
            launch_arguments: vec![OsString::from("--service")],
            dependencies: vec![OsString::from("Tcpip")],
            account_name: None, // LocalSystem
            account_password: None,
        };

        let service = manager
            .create_service(&service_info, ServiceAccess::CHANGE_CONFIG)
            .map_err(|e| {
                format!(
                    "Failed to create service: {} (Are you running as Administrator?)",
                    e
                )
            })?;

        // Set description
        service
            .set_description(SERVICE_DESCRIPTION)
            .map_err(|e| format!("Failed to set service description: {}", e))?;

        println!("Windows Service '{}' installed successfully!", SERVICE_NAME);
        println!("  Display Name: {}", SERVICE_DISPLAY_NAME);
        println!("  Executable: {}", exe_path.display());
        println!();
        println!("To start: sc start {}", SERVICE_NAME);
        println!("To stop:  sc stop {}", SERVICE_NAME);
        println!("To check: sc query {}", SERVICE_NAME);

        Ok(())
    }

    #[cfg(not(windows))]
    {
        Err("Windows service management is only available on Windows")
    }
}

/// Uninstall the Windows Service
pub fn uninstall_service() -> ServiceResult<()> {
    #[cfg(windows)]
    {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;

        let service = manager.open_service(
            SERVICE_NAME,
            ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS,
        )?;

        // Stop service if running
        let status = service.query_status()?;
        if status.current_state != ServiceState::Stopped {
            service.stop()?;
            // Wait for service to stop
            std::thread::sleep(Duration::from_secs(3));
        }

        service.delete()?;

        println!(
            "Windows Service '{}' uninstalled successfully!",
            SERVICE_NAME
        );

        Ok(())
    }

    #[cfg(not(windows))]
    {
        Err("Windows service management is only available on Windows")
    }
}

/// Start the Windows Service
pub fn start_service() -> ServiceResult<()> {
    #[cfg(windows)]
    {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;

        let service = manager.open_service(SERVICE_NAME, ServiceAccess::START)?;
        service.start(&[OsString::from("")])?;

        println!("Service '{}' started.", SERVICE_NAME);
        Ok(())
    }

    #[cfg(not(windows))]
    {
        Err("Windows service management is only available on Windows")
    }
}

/// Stop the Windows Service
pub fn stop_service() -> ServiceResult<()> {
    #[cfg(windows)]
    {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;

        let service = manager.open_service(SERVICE_NAME, ServiceAccess::STOP)?;
        service.stop()?;

        println!("Service '{}' stopped.", SERVICE_NAME);
        Ok(())
    }

    #[cfg(not(windows))]
    {
        Err("Windows service management is only available on Windows")
    }
}

/// Get service status
pub fn status_service() -> ServiceResult<String> {
    #[cfg(windows)]
    {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;

        match manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS) {
            Ok(service) => {
                let status = service.query_status()?;
                match status.current_state {
                    ServiceState::Running => Ok("Running".to_string()),
                    ServiceState::Stopped => Ok("Stopped".to_string()),
                    ServiceState::StartPending => Ok("Start Pending".to_string()),
                    ServiceState::StopPending => Ok("Stop Pending".to_string()),
                    ServiceState::ContinuePending => Ok("Continue Pending".to_string()),
                    ServiceState::PausePending => Ok("Pause Pending".to_string()),
                    ServiceState::Paused => Ok("Paused".to_string()),
                }
            }
            Err(_) => Ok("Not installed".to_string()),
        }
    }

    #[cfg(not(windows))]
    {
        Err("Windows service management is only available on Windows")
    }
}

// ============================================================================
// Windows Service Entry Point
// ============================================================================

#[cfg(windows)]
define_windows_service!(ffi_service_main, service_main);

/// This is called by Windows SCM when the service is started
#[cfg(windows)]
fn service_main(arguments: Vec<OsString>) {
    if let Err(e) = run_service(arguments) {
        eprintln!("Service error: {}", e);
    }
}

#[cfg(windows)]
fn run_service(_arguments: Vec<OsString>) -> ServiceResult<()> {
    use std::sync::Arc;

    // Create a channel to receive stop events
    let (shutdown_tx, shutdown_rx) = mpsc::channel();

    // Status handle needs to be shared between event handler and main thread
    // so we can report StopPending immediately when Stop is received
    let status_handle_for_event: Arc<std::sync::Mutex<Option<service_control_handler::ServiceStatusHandle>>> =
        Arc::new(std::sync::Mutex::new(None));
    let status_handle_clone = status_handle_for_event.clone();

    // Create service control handler
    let shutdown_tx_clone = shutdown_tx.clone();
    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                // FIX #1: Report StopPending IMMEDIATELY to SCM
                // This prevents SCM from killing the service during graceful shutdown
                if let Ok(guard) = status_handle_clone.lock() {
                    if let Some(ref handle) = *guard {
                        let _ = handle.set_service_status(ServiceStatus {
                            service_type: SERVICE_TYPE,
                            current_state: ServiceState::StopPending,
                            controls_accepted: ServiceControlAccept::empty(),
                            exit_code: ServiceExitCode::Win32(0),
                            checkpoint: 1,
                            wait_hint: Duration::from_secs(30), // Give 30s for graceful shutdown
                            process_id: None,
                        });
                    }
                }
                let _ = shutdown_tx_clone.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    // Register service control handler
    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

    // Store status handle for event handler to use
    if let Ok(mut guard) = status_handle_for_event.lock() {
        *guard = Some(status_handle.clone());
    }

    // Report service as running
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::from_secs(30),
        process_id: None,
    })?;

    // Run the actual server
    // We use a separate thread for the async runtime
    let server_handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
        rt.block_on(async {
            crate::http_server::run_http_server_with_shutdown(shutdown_rx).await;
        });
    });

    // Wait for the server to finish with timeout
    // FIX #1: Don't block forever - use join with periodic checkpoint updates
    let shutdown_start = std::time::Instant::now();
    const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
    const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(5);

    let mut checkpoint = 2u32;
    loop {
        // Check if server thread finished
        if server_handle.is_finished() {
            let _ = server_handle.join();
            break;
        }

        // Check timeout
        if shutdown_start.elapsed() > SHUTDOWN_TIMEOUT {
            eprintln!("Service shutdown timeout after {:?}, forcing stop", SHUTDOWN_TIMEOUT);
            // Thread will be killed when process exits
            break;
        }

        // Update checkpoint to show SCM we're still working
        let _ = status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::StopPending,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint,
            wait_hint: Duration::from_secs(10),
            process_id: None,
        });
        checkpoint = checkpoint.saturating_add(1);

        std::thread::sleep(CHECKPOINT_INTERVAL);
    }

    // Report service as stopped
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::from_secs(0),
        process_id: None,
    })?;

    Ok(())
}

/// Start as Windows service (called from main when --service flag is present)
#[cfg(windows)]
pub fn run_as_service() -> ServiceResult<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)?;
    Ok(())
}

#[cfg(not(windows))]
pub fn run_as_service() -> ServiceResult<()> {
    Err("Windows service mode is only available on Windows")
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
