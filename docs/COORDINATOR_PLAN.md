# IronBase Coordinator - Distributed Query Layer

> **Státusz**: TERV - Későbbi implementációra vár

## Cél

Coordinator komponens ami N darab MCP szervert kezel és egységes API-t ad:
- **Scatter-Gather**: Query ALL szerverekre, eredmények aggregálása
- **Index Lookup**: Célzott query specifikus szerverekre
- **Fault Tolerant**: Timeout + partial failure kezelés

## Use Cases

| Pattern | Példa | Működés |
|---------|-------|---------|
| Scatter-Gather | "Hibák mindenhonnan" | Broadcast → Aggregate |
| Index Lookup | "Hol járt ABC-123?" | Index query → Targeted fetch |

## Architektúra

```
┌─────────────────────────────────────────────────────────────┐
│                    COORDINATOR                               │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │ Fleet Mgmt  │  │   Router    │  │   Index Server      │  │
│  │ (endpoints) │  │ (scatter/   │  │ (entity → servers)  │  │
│  │             │  │  gather)    │  │                     │  │
│  └─────────────┘  └─────────────┘  └─────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
         │ parallel HTTP (reqwest, connection pooling)
         ▼
┌────┐ ┌────┐ ┌────┐ ┌────┐     ┌────┐
│ M1 │ │ M2 │ │ M3 │ │ M4 │ ... │ MN │  (MCP Servers)
└────┘ └────┘ └────┘ └────┘     └────┘
```

## Optimális pattern: "Routing Only"

**Coordinator terhelés minimalizálása:**

```
Client ──→ Coordinator (csak index lookup, ~1-2ms)
       └──→ M1 ──────────────────────→ Client
       └──→ M2 ──────────────────────→ Client  (parallel, direkt)
       └──→ M3 ──────────────────────→ Client
```

Coordinator CSAK:
1. **Index kezelés** - entity → servers mapping
2. **Fleet health** - melyik szerver él

Kliens csinálja:
- Parallel HTTP hívásokat (reqwest connection pool)
- Eredmények aggregálását

## API Design

### 1. Fleet Management
```rust
coordinator.add_server("budapest", "http://192.168.1.10:8080/mcp");
coordinator.add_server("gyor", "http://192.168.1.11:8080/mcp");
coordinator.remove_server("budapest");
coordinator.list_servers() -> Vec<ServerInfo>
coordinator.health_check() -> HashMap<String, bool>
```

### 2. Scatter-Gather (broadcast)
```rust
let results = coordinator.scatter_gather(
    "find",
    json!({"collection": "errors", "query": {"level": "error"}}),
    ScatterOptions {
        timeout: Duration::from_secs(5),
        require_all: false,  // partial results OK
    }
).await;

// Eredmény:
ScatterResult {
    succeeded: 498,
    failed: 2,
    results: vec![
        ServerResult { server: "budapest", data: [...] },
        ServerResult { server: "gyor", data: [...] },
    ],
    errors: vec![
        ServerError { server: "pecs", error: "timeout" },
    ]
}
```

### 3. Index-based Lookup
```rust
// Entitás regisztrálása (méréskor)
coordinator.index_register("plate", "ABC-123", "budapest").await;

// Entitás keresése
let servers = coordinator.index_lookup("plate", "ABC-123").await;
// -> ["budapest", "gyor", "pecs"]

// Célzott query csak az érintett szerverekre
let results = coordinator.indexed_query(
    "plate", "ABC-123",
    "find",
    json!({"collection": "measurements", "query": {"plate": "ABC-123"}})
).await;
```

## Implementáció

### Új crate: `ironbase-coordinator/`

```
ironbase-coordinator/
├── Cargo.toml
├── src/
│   ├── lib.rs           # Public API
│   ├── fleet.rs         # Server management
│   ├── scatter.rs       # Scatter-gather logic
│   ├── index.rs         # Index server (entity → servers)
│   ├── client.rs        # HTTP client (reqwest)
│   └── error.rs         # Error types
```

### Cargo.toml
```toml
[package]
name = "ironbase-coordinator"
version = "0.1.0"

[dependencies]
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tracing = "0.1"

# Index storage (optional - saját IronBase)
ironbase-core = { path = "../ironbase-core", optional = true }

[features]
default = ["local-index"]
local-index = ["ironbase-core"]
```

### Core structs

```rust
pub struct Coordinator {
    fleet: Fleet,
    index: Option<IndexServer>,
    client: reqwest::Client,
}

pub struct Fleet {
    servers: HashMap<String, ServerEndpoint>,
}

pub struct ServerEndpoint {
    name: String,
    url: String,
    healthy: AtomicBool,
    last_check: AtomicU64,
}

pub struct ScatterOptions {
    pub timeout: Duration,
    pub require_all: bool,
    pub max_concurrent: usize,
}

pub struct ScatterResult<T> {
    pub succeeded: usize,
    pub failed: usize,
    pub results: Vec<ServerResult<T>>,
    pub errors: Vec<ServerError>,
}
```

## Fázisok

### Fázis 1: Alap struktúra
- [ ] `ironbase-coordinator/` crate létrehozás
- [ ] Fleet management (add/remove/list servers)
- [ ] Health check endpoint

### Fázis 2: Scatter-Gather
- [ ] HTTP client (reqwest + connection pooling)
- [ ] Parallel query végrehajtás
- [ ] Timeout + error handling
- [ ] Result aggregation

### Fázis 3: Index Server
- [ ] Entity registration API
- [ ] Lookup API
- [ ] Targeted query (index → scatter subset)

### Fázis 4: MCP Integration (opcionális)
- [ ] `fleet_scatter`, `fleet_lookup` tools
- [ ] Integration az mcp-server-be

### Fázis 5: Tesztek + Dokumentáció
- [ ] Unit tesztek (mock HTTP)
- [ ] Integration teszt (több MCP szerver)
- [ ] README.md

## Nyitott kérdések

1. **Index storage**: Saját IronBase instance vagy in-memory HashMap?
2. **MCP integration**: Kell-e az mcp-server-be tool-ként?
3. **CLI**: Kell-e standalone binary?

## Technológiai döntések

| Kérdés | Döntés | Indoklás |
|--------|--------|----------|
| Protokoll | HTTP/JSON-RPC | MCP-kompatibilis, reqwest poolingol |
| Connection | reqwest pool | Beépített pool, nem kell kézzel kezelni |
| Terhelés | Routing only | Coordinator csak index, kliens aggregál |
