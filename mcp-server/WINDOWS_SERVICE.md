# Windows Service Telepítési Útmutató

## 1. Build (Windows-on)

```powershell
cd mcp-server
cargo build --release --target x86_64-pc-windows-msvc
```

## 2. Telepítés (Admin PowerShell)

```powershell
# Telepítés
.\target\release\mcp-ironbase-server.exe install

# Indítás
sc start IronBaseService

# Státusz ellenőrzés
sc query IronBaseService
```

## 3. Konfiguráció

Adatbázis helye (automatikus):
- **User mód:** `%LOCALAPPDATA%\IronBase\data\ironbase_data.mlite`
- **Service mód:** `C:\ProgramData\IronBase\ironbase_data.mlite`

Vagy környezeti változóval:
```powershell
# System Environment Variables-ban:
IRONBASE_PATH = "D:\MyData\ironbase.mlite"
IRONBASE_ADMIN_KEY = "titkos_kulcs"
MCP_PORT = "8080"
```

## 4. Egyéb parancsok

```powershell
# Leállítás
sc stop IronBaseService

# Eltávolítás
.\mcp-ironbase-server.exe uninstall

# Státusz (CLI-ből)
.\mcp-ironbase-server.exe status
```

## 5. Hibaelhárítás

```powershell
# Event Viewer (egyelőre korlátozott)
eventvwr.msc → Windows Logs → Application

# Service log (ha van)
Get-Content C:\ProgramData\IronBase\*.log

# Kézi teszt (service nélkül)
.\mcp-ironbase-server.exe --port 8080
```
