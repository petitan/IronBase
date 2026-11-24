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
    VectorSearch,
    Rag,
}

/// The open modal and the state it owns: it is created when the modal opens
/// and dropped when it closes, so a modal never shows a previous opening's
/// state. `Plain` is for the modals without state of their own (or whose
/// state is view state kept in `App`); `kind` names the modal either way.
// One value of it lives in `App` (one modal is open at a time), so its size
// is the largest modal state's either way; boxing each variant would add an
// indirection to every access for nothing
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum OpenModal {
    Plain(Modal),
    NewCollection(crate::state::NewCollectionState),
    ServerInfo(crate::modals::server_info::ServerInfoState),
    Update(crate::modals::update::UpdateState),
    Export(crate::state::ExportState),
    Fulltext(crate::state::FulltextState),
    Database(crate::state::DatabaseState),
    Insert(crate::state::InsertState),
    Query(crate::state::QueryState),
    ApiKey(crate::modals::api_key::ApiKeyState),
    Acl(crate::modals::acl::AclState),
    Listener(crate::modals::listener::ListenerState),
    Index(crate::state::index::IndexState),
    Vector(crate::state::vector::VectorSearchState),
    Script(crate::state::script::ScriptState),
    Rag(crate::state::rag::RagState),
    /// A confirm dialog; `previous` is the modal it was opened from, given
    /// back as it was when the dialog closes (`None`: back to the panes)
    Confirm {
        state: crate::modals::confirm::ConfirmState,
        previous: Option<Box<OpenModal>>,
    },
}

impl OpenModal {
    /// A modal of `kind` with a fresh, empty state (the `open_*` functions
    /// fill theirs in; this is what a modal starts from without them)
    pub fn new_default(kind: Modal) -> Self {
        match kind {
            Modal::NewCollection => OpenModal::NewCollection(Default::default()),
            Modal::ServerInfo => OpenModal::ServerInfo(Default::default()),
            Modal::Update => OpenModal::Update(Default::default()),
            Modal::Export => OpenModal::Export(Default::default()),
            Modal::Fulltext => OpenModal::Fulltext(Default::default()),
            Modal::Database => OpenModal::Database(Default::default()),
            Modal::Insert => OpenModal::Insert(Default::default()),
            Modal::Query => OpenModal::Query(Default::default()),
            Modal::ApiKey => OpenModal::ApiKey(Default::default()),
            Modal::Acl => OpenModal::Acl(Default::default()),
            Modal::Listener => OpenModal::Listener(Default::default()),
            Modal::Index => OpenModal::Index(Default::default()),
            Modal::VectorSearch => OpenModal::Vector(Default::default()),
            Modal::Script => OpenModal::Script(Default::default()),
            Modal::Rag => OpenModal::Rag(Default::default()),
            Modal::Confirm => OpenModal::Confirm {
                state: Default::default(),
                previous: None,
            },
            Modal::Search | Modal::Actions | Modal::Help | Modal::Filter | Modal::ErrorDetail => {
                OpenModal::Plain(kind)
            }
        }
    }

    pub fn kind(&self) -> Modal {
        match self {
            OpenModal::Plain(kind) => *kind,
            OpenModal::NewCollection(_) => Modal::NewCollection,
            OpenModal::ServerInfo(_) => Modal::ServerInfo,
            OpenModal::Update(_) => Modal::Update,
            OpenModal::Export(_) => Modal::Export,
            OpenModal::Fulltext(_) => Modal::Fulltext,
            OpenModal::Database(_) => Modal::Database,
            OpenModal::Insert(_) => Modal::Insert,
            OpenModal::Query(_) => Modal::Query,
            OpenModal::ApiKey(_) => Modal::ApiKey,
            OpenModal::Acl(_) => Modal::Acl,
            OpenModal::Listener(_) => Modal::Listener,
            OpenModal::Index(_) => Modal::Index,
            OpenModal::Vector(_) => Modal::VectorSearch,
            OpenModal::Script(_) => Modal::Script,
            OpenModal::Rag(_) => Modal::Rag,
            OpenModal::Confirm { .. } => Modal::Confirm,
        }
    }
}

/// The open `$variant` modal's state, borrowing only `$app.open_modal` (so
/// `$app.db` and the other fields stay usable next to it); the caller returns
/// when that modal is not open
macro_rules! open_state {
    ($app:expr, $variant:ident) => {
        match &mut $app.open_modal {
            Some($crate::state::OpenModal::$variant(state)) => state,
            _ => return,
        }
    };
}
pub(crate) use open_state;

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
