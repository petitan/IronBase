// Graceful Shutdown Module
//
// Provides cross-platform signal handling for graceful server shutdown.
// Supports SIGTERM (Unix), SIGINT (Ctrl+C), and Windows service events.

use tokio::sync::broadcast;

/// Shutdown handle for coordinating graceful shutdown across tasks
#[derive(Clone)]
pub struct ShutdownHandle {
    tx: broadcast::Sender<()>,
}

impl ShutdownHandle {
    /// Create a new shutdown handle and receiver
    pub fn new() -> (Self, broadcast::Receiver<()>) {
        let (tx, rx) = broadcast::channel(1);
        (Self { tx }, rx)
    }

    /// Subscribe to shutdown notifications
    #[allow(dead_code)]
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.tx.subscribe()
    }

    /// Trigger shutdown (used by Windows service handler)
    #[allow(dead_code)]
    pub fn trigger(&self) {
        let _ = self.tx.send(());
    }
}

impl Default for ShutdownHandle {
    fn default() -> Self {
        Self::new().0
    }
}

/// Wait for shutdown signal (Ctrl+C or SIGTERM)
///
/// This function will block until a shutdown signal is received.
/// On Unix, it handles both SIGINT (Ctrl+C) and SIGTERM.
/// On Windows, it handles Ctrl+C.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Received Ctrl+C, initiating graceful shutdown...");
        }
        _ = terminate => {
            tracing::info!("Received SIGTERM, initiating graceful shutdown...");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_shutdown_handle_trigger() {
        let (handle, mut rx) = ShutdownHandle::new();

        // Trigger in background
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            handle.trigger();
        });

        // Should receive shutdown signal
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            rx.recv()
        ).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_shutdown_handle_subscribe() {
        let (handle, _rx) = ShutdownHandle::new();
        let mut rx2 = handle.subscribe();

        // Trigger in background
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            handle.trigger();
        });

        // Subscriber should also receive shutdown signal
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            rx2.recv()
        ).await;

        assert!(result.is_ok());
    }
}
