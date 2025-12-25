//! Common types and enums for TUI state

/// Active pane in the 3-panel layout
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pane {
    #[default]
    Collections,
    Documents,
    Detail,
}

impl Pane {
    /// Get next pane (Tab)
    pub fn next(&self) -> Self {
        match self {
            Pane::Collections => Pane::Documents,
            Pane::Documents => Pane::Detail,
            Pane::Detail => Pane::Collections,
        }
    }

    /// Get previous pane (Shift+Tab)
    pub fn prev(&self) -> Self {
        match self {
            Pane::Collections => Pane::Detail,
            Pane::Documents => Pane::Collections,
            Pane::Detail => Pane::Documents,
        }
    }
}

/// Currently active modal (if any)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modal {
    Search,
    Actions,
    Help,
    Confirm,
    Insert,
    Index,
    Query,
    Export,
    Filter,
    ErrorDetail,
    NewCollection,
    Script,
    ServerInfo,
    Update,
    Database,
    ApiKey,
    Fulltext,
    Acl,
    Listener,
}

/// Connection type for permission checks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionType {
    /// Loopback (127.0.0.1, localhost, ::1)
    Localhost,
    /// Private network (10.x, 172.16-31.x, 192.168.x)
    Internal,
    /// Public network
    External,
    /// Unknown (stdio or not determined)
    #[default]
    Unknown,
}

impl ConnectionType {
    /// Detect connection type from URL
    pub fn from_url(url: &str) -> Self {
        let url_lower = url.to_lowercase();
        if url_lower.contains("127.0.0.1")
            || url_lower.contains("localhost")
            || url_lower.contains("[::1]")
        {
            Self::Localhost
        } else if url_lower.contains("192.168.")
            || url_lower.contains("10.")
            || url_lower.starts_with("http://172.")
        {
            Self::Internal
        } else if url_lower.starts_with("http://") || url_lower.starts_with("https://") {
            Self::External
        } else {
            Self::Unknown
        }
    }

    /// Check if this is a localhost connection
    pub fn is_localhost(&self) -> bool {
        matches!(self, Self::Localhost | Self::Unknown) // Unknown = stdio = localhost
    }
}

/// Search mode - collections or document content
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchMode {
    #[default]
    Collections,
    Document,
}

/// Editor mode - insert or edit
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditorMode {
    #[default]
    Insert,
    Edit,
}

/// Database open/create mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DatabaseMode {
    #[default]
    Open,
    Create,
}
