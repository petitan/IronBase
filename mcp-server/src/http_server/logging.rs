//! Logging utilities for HTTP server
//!
//! Provides synchronous file writer for crash-safe logging.

/// A file writer that calls fsync() after every write.
/// Use for crash debugging when you need guaranteed log persistence.
/// WARNING: Significantly slower than buffered logging.
#[derive(Clone)]
pub struct SyncFileWriter {
    file: std::sync::Arc<std::sync::Mutex<std::fs::File>>,
}

impl SyncFileWriter {
    pub fn new(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self {
            file: std::sync::Arc::new(std::sync::Mutex::new(file)),
        })
    }
}

impl std::io::Write for SyncFileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Handle mutex poisoning gracefully - recover the data and continue
        let mut file = match self.file.lock() {
            Ok(f) => f,
            Err(poisoned) => poisoned.into_inner(),
        };
        let n = file.write(buf)?;
        file.sync_all()?; // fsync after every write
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        // Handle mutex poisoning gracefully - recover the data and continue
        let mut file = match self.file.lock() {
            Ok(f) => f,
            Err(poisoned) => poisoned.into_inner(),
        };
        file.flush()?;
        file.sync_all()
    }
}

// tracing_subscriber MakeWriter implementation
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SyncFileWriter {
    type Writer = SyncFileWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
