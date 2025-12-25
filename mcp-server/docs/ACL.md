# Jogosultság kezelés (ACL)

## Permissions flag-ek

| Flag | Leírás |
|------|--------|
| `read` | Lekérdezés (find, count, list, aggregate) |
| `write` | Adatmódosítás (insert, update, delete) |
| `admin` | Struktúra módosítás (create_index, drop_collection, schema) |

## Hierarchia

| Van joga | Tehet |
|----------|-------|
| `admin: true` | read + write + admin |
| `write: true` | read + write |
| `read: true` | read |

## Kliens típusok (InterfaceType)

| Típus | IP tartomány |
|-------|--------------|
| `localhost` | 127.0.0.1, ::1 |
| `internal` | 10.x.x.x, 172.16-31.x.x, 192.168.x.x |
| `external` | minden más |

## Builtin ACL szabályok

| Collection | Localhost | Internal | External |
|------------|-----------|----------|----------|
| `_system.scripts` | read+write+admin | read | read |
| `_system.acl` | read+write+admin | - | - |
| `_system.listeners` | read+write+admin | - | - |
| `_system.api_keys` | read+write+admin | - | - |
| `*` (minden más) | read+write+admin | read+write | read |

## Tool-ok és jogosultságok

| Tool | Szükséges jog | Localhost-only |
|------|---------------|----------------|
| `find`, `count_documents`, `aggregate`, `collection_list`, `fulltext_search`, `fuzzy_search` | read | nem |
| `insert_*`, `update_*`, `delete_*` | write | nem |
| `index_create`, `collection_drop`, `schema_set` | admin | nem |
| `script_run`, `script_exec` | read | nem |
| `script_save`, `script_delete`, `script_rollback` | admin | igen |
| `acl_list`, `acl_get` | read | nem |
| `acl_set`, `acl_delete` | admin | igen |
| `listener_list`, `listener_get` | read | nem |
| `listener_add`, `listener_delete`, `listener_enable/disable` | admin | igen |
| `admin_list_all_collections` | admin | igen |
| `admin_create_system_collection` | admin | igen |
| `admin_set_collection_flags` | admin | igen |
| `admin_drop_protected` | admin | igen |
| `admin_apikey_*` | admin | igen |

## ACL beállítás példa

```json
{
    "tool": "acl_set",
    "arguments": {
        "collection": "users",
        "rules": [
            { "principal": "interface:internal", "permissions": "read,write" },
            { "principal": "interface:external", "permissions": "read" },
            { "principal": "apikey:partner_key", "permissions": "read,write" }
        ]
    }
}
```

## Principal típusok

| Típus | Formátum | Példa |
|-------|----------|-------|
| Interface | `interface:<type>` | `interface:internal` |
| API kulcs | `apikey:<key>` | `apikey:abc123` |
| IP cím | `ip:<address>` | `ip:192.168.1.50` |
| IP tartomány | `iprange:<cidr>` | `iprange:192.168.1.0/24` |
| Bárki | `anyone` | `anyone` |

## Admin Key

Az `admin_*` tool-ok további védelmet kapnak az `IRONBASE_ADMIN_KEY` környezeti változóval:

1. **Localhost követelmény**: Csak localhost-ról hívhatók
2. **Admin key paraméter**: Minden hívásnak tartalmaznia kell az `admin_key` paramétert

```json
{
    "tool": "admin_apikey_create",
    "arguments": {
        "admin_key": "your-secret-admin-key",
        "name": "partner_api"
    }
}
```

Ha az `IRONBASE_ADMIN_KEY` nincs beállítva, az admin tool-ok le vannak tiltva.
