# TUI ACL Frissítési Terv

## Összefoglaló

A TUI-t frissíteni kell az új ACL (Access Control List) rendszerhez:
- Kapcsolat típus megjelenítése (localhost/internal/external)
- ACL kezelő modal
- Listener kezelő modal
- API Key modal frissítése localhost figyelmeztetéssel
- Jogosultsági hibák kezelése

---

## 1. MCP Client bővítés (`src/mcp/client.rs`)

### 1.1 Új ACL metódusok

```rust
// === ACL Management ===

/// List all ACL rules
pub async fn acl_list(&self) -> McpResult<Vec<Value>>

/// Get ACL for a specific collection
pub async fn acl_get(&self, collection: &str) -> McpResult<Value>

/// Set ACL for a collection (localhost only)
pub async fn acl_set(&self, collection: &str, rules: Vec<Value>) -> McpResult<bool>

/// Delete ACL for a collection (localhost only)
pub async fn acl_delete(&self, collection: &str) -> McpResult<bool>
```

### 1.2 Új Listener metódusok

```rust
// === Listener Management ===

/// List all listeners
pub async fn listener_list(&self) -> McpResult<Vec<Value>>

/// Get listener by ID
pub async fn listener_get(&self, id: &str) -> McpResult<Value>

/// Add new listener (localhost only)
pub async fn listener_add(&self, config: Value) -> McpResult<String>

/// Delete listener (localhost only)
pub async fn listener_delete(&self, id: &str) -> McpResult<bool>

/// Enable listener (localhost only)
pub async fn listener_enable(&self, id: &str) -> McpResult<bool>

/// Disable listener (localhost only)
pub async fn listener_disable(&self, id: &str) -> McpResult<bool>
```

---

## 2. Új Modal: ACL Kezelés (`src/modals/acl.rs`)

### 2.1 Struktúrák

```rust
pub struct AclRule {
    pub principal: String,      // "interface:internal", "apikey:xxx", etc.
    pub permissions: String,    // "read", "read,write", "read,write,admin"
}

pub struct CollectionAcl {
    pub collection: String,
    pub rules: Vec<AclRule>,
    pub is_builtin: bool,       // Built-in rules can't be deleted
}

pub enum AclModalMode {
    List,           // List all ACL rules
    ViewCollection, // View rules for one collection
    EditRule,       // Edit/create rule
    Confirm,        // Confirm delete
}

pub struct AclState {
    pub mode: AclModalMode,
    pub acls: Vec<CollectionAcl>,
    pub selected: usize,
    pub selected_rule: usize,
    pub edit_collection: String,
    pub edit_principal: String,
    pub edit_permissions: String,
    pub error: Option<String>,
    pub success: Option<String>,
    pub is_localhost: bool,     // Can only edit if localhost
}
```

### 2.2 UI Tervezés

```
┌─────────────────── ACL Rules ───────────────────┐
│ [j/k] Nav  [Enter] View  [n] New  [d] Delete    │
│ [Esc] Close                                     │
│                                                 │
│ ┌─────────────────────────────────────────────┐ │
│ │ Collection          │ Rules                 │ │
│ ├─────────────────────┼───────────────────────┤ │
│ │ _system.scripts     │ 3 rules (builtin)     │ │
│ │ _system.acl         │ 1 rule  (builtin)     │ │
│ │ _system.listeners   │ 1 rule  (builtin)     │ │
│ │ users               │ 2 rules               │ │
│ │ orders              │ 1 rule                │ │
│ │ * (default)         │ 3 rules (builtin)     │ │
│ └─────────────────────┴───────────────────────┘ │
│                                                 │
│ ⚠ Editing requires localhost connection         │
└─────────────────────────────────────────────────┘
```

Collection részletei:
```
┌──────────── ACL: users ────────────┐
│ [j/k] Nav  [n] New  [e] Edit       │
│ [d] Delete  [Esc] Back             │
│                                    │
│ Principal            Permissions   │
│ ──────────────────────────────────│
│ interface:localhost  read,write,admin │
│ interface:internal   read,write    │
│ interface:external   read          │
│                                    │
└────────────────────────────────────┘
```

---

## 3. Új Modal: Listener Kezelés (`src/modals/listener.rs`)

### 3.1 Struktúrák

```rust
pub struct ListenerInfo {
    pub id: String,
    pub bind: String,           // "0.0.0.0:8080"
    pub tls: bool,
    pub enabled: bool,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
}

pub enum ListenerModalMode {
    List,
    Add,
    Confirm,
}

pub struct ListenerState {
    pub mode: ListenerModalMode,
    pub listeners: Vec<ListenerInfo>,
    pub selected: usize,
    pub add_bind: String,
    pub add_tls: bool,
    pub add_cert: String,
    pub add_key: String,
    pub confirm_action: Option<ListenerAction>,
    pub error: Option<String>,
    pub success: Option<String>,
    pub is_localhost: bool,
}

pub enum ListenerAction {
    Delete,
    Enable,
    Disable,
}
```

### 3.2 UI Tervezés

```
┌────────────────── Listeners ──────────────────┐
│ [j/k] Nav  [n] New  [e] Enable  [x] Disable   │
│ [d] Delete  [Esc] Close                       │
│                                               │
│ ID              Bind              TLS  Status │
│ ─────────────────────────────────────────────│
│ internal        192.168.1.100:8080  ✗  Active │
│ external        0.0.0.0:443         ✓  Active │
│ backup          0.0.0.0:8081        ✗  Disabled│
│                                               │
│ ⚠ Changes require server restart              │
│ ⚠ Editing requires localhost connection       │
└───────────────────────────────────────────────┘
```

---

## 4. API Key Modal frissítés (`src/modals/api_key.rs`)

### 4.1 Változások

1. **Localhost figyelmeztetés hozzáadása:**
```rust
// Ha nem localhost, mutassuk a figyelmeztetést
if !state.is_localhost {
    Span::styled(
        "⚠ Admin operations require localhost connection",
        Style::default().fg(theme.warning)
    )
}
```

2. **Hibaüzenet kezelés javítása:**
```rust
// Új ACL hibák kezelése
if error.contains("can only be called from localhost") {
    "This operation requires localhost connection"
} else if error.contains("Access denied") {
    // Parse and show specific permission error
}
```

---

## 5. App állapot bővítés (`src/app.rs`)

### 5.1 Új mezők

```rust
pub struct App {
    // ... existing fields ...

    /// Connection type (localhost/internal/external)
    pub connection_type: ConnectionType,

    /// ACL modal state
    pub acl_state: AclState,

    /// Listener modal state
    pub listener_state: ListenerState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionType {
    Localhost,
    Internal,
    External,
    Unknown,  // Stdio mode or not yet determined
}
```

### 5.2 Új metódusok

```rust
impl App {
    /// Detect connection type from URL
    pub fn detect_connection_type(url: &str) -> ConnectionType {
        if url.contains("127.0.0.1") || url.contains("localhost") || url.contains("::1") {
            ConnectionType::Localhost
        } else if url.contains("192.168.") || url.contains("10.") || url.starts_with("172.") {
            ConnectionType::Internal
        } else {
            ConnectionType::External
        }
    }

    /// Load ACL list
    pub async fn load_acl_list(&mut self) -> Result<()>

    /// Load listeners
    pub async fn load_listeners(&mut self) -> Result<()>

    /// Set ACL for collection
    pub async fn set_acl(&mut self, collection: &str, rules: Vec<AclRule>) -> Result<()>

    /// Delete ACL for collection
    pub async fn delete_acl(&mut self, collection: &str) -> Result<()>

    // ... listener methods ...
}
```

---

## 6. Main frissítés (`src/main.rs`)

### 6.1 Új billentyűk

| Billentyű | Funkció |
|-----------|---------|
| `Shift+A` | ACL kezelő modal megnyitása |
| `Shift+L` | Listener kezelő modal megnyitása |
| `Shift+K` | API Key modal (már létezik) |

### 6.2 Új Modal típusok

```rust
pub enum ActiveModal {
    // ... existing ...
    Acl,
    Listener,
}
```

### 6.3 Status bar bővítés

```rust
// Show connection type in status bar
let conn_type = match app.connection_type {
    ConnectionType::Localhost => Span::styled("LOCAL", Style::default().fg(theme.success)),
    ConnectionType::Internal => Span::styled("LAN", Style::default().fg(theme.accent)),
    ConnectionType::External => Span::styled("WAN", Style::default().fg(theme.warning)),
    ConnectionType::Unknown => Span::styled("STDIO", Style::default().fg(theme.muted)),
};
```

---

## 7. Fájl struktúra

```
src/modals/
├── mod.rs          # Add: pub mod acl; pub mod listener;
├── acl.rs          # NEW - ACL management modal
├── listener.rs     # NEW - Listener management modal
├── api_key.rs      # UPDATE - Add localhost warning
└── ...
```

---

## 8. Implementációs sorrend

| # | Feladat | Fájl(ok) | Becslés |
|---|---------|----------|---------|
| 1 | MCP client ACL metódusok | `mcp/client.rs` | egyszerű |
| 2 | MCP client Listener metódusok | `mcp/client.rs` | egyszerű |
| 3 | Connection type detection | `app.rs`, `main.rs` | egyszerű |
| 4 | ACL modal struktúrák | `modals/acl.rs` | közepes |
| 5 | ACL modal renderelés | `modals/acl.rs` | közepes |
| 6 | ACL modal kezelés | `main.rs`, `app.rs` | közepes |
| 7 | Listener modal struktúrák | `modals/listener.rs` | közepes |
| 8 | Listener modal renderelés | `modals/listener.rs` | közepes |
| 9 | Listener modal kezelés | `main.rs`, `app.rs` | közepes |
| 10 | API Key modal localhost warning | `modals/api_key.rs` | egyszerű |
| 11 | Status bar connection type | `main.rs` | egyszerű |
| 12 | Tesztelés | - | - |

---

## 9. Megjegyzések

- A **Stdio mód** esetén a kapcsolat mindig localhost-nak számít (a TUI közvetlenül spawn-olja a szervert)
- Az ACL és Listener módosítások **csak localhost**-ról működnek
- A TUI megmutatja a figyelmeztetést, ha nem localhost a kapcsolat
- A hibakezelés informatív üzeneteket ad az ACL hibákról
