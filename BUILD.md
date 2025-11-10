# MongoLite - Build és Telepítési Útmutató

## 🔧 Előfeltételek

### 1. Rust Telepítése

#### Linux / macOS
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

#### Windows
1. Töltsd le a Rust installert: https://rustup.rs/
2. Futtatd a `rustup-init.exe` fájlt
3. **FONTOS**: A MSVC toolchain automatikusan települ, de szükséges a **Microsoft C++ Build Tools**

**Microsoft C++ Build Tools telepítése:**
- Letöltés: https://visualstudio.microsoft.com/visual-cpp-build-tools/
- Vagy telepítsd a Visual Studio-t a "Desktop development with C++" workload-dal
- Minimális követelmények:
  - MSVC v142+ (vagy újabb)
  - Windows 10 SDK

Ellenőrzés:
```bash
rustc --version
cargo --version
```

### 2. Python Telepítése
```bash
# Minimum: Python 3.8
python --version
# vagy
python3 --version
```

### 3. Maturin Telepítése
```bash
pip install maturin
# vagy
pip3 install maturin
```

## 🚀 Build Folyamat

### Development Build (Gyors, Debug)
```bash
cd mongolite_project

# Build és install Python package-ként
maturin develop

# Sikeres build után:
python example.py
```

### Release Build (Optimalizált)
```bash
# Teljes optimalizálás
maturin build --release

# Wheel fájl (platform szerint):
ls target/wheels/
# Linux:   mongolite-0.2.0-cp38-abi3-linux_x86_64.whl
# Windows: mongolite-0.2.0-cp38-abi3-win_amd64.whl
# macOS:   mongolite-0.2.0-cp38-abi3-macosx_11_0_universal2.whl
```

### Csak Rust Build (Python nélkül)
```bash
# Library build
cargo build --release

# Tesztek futtatása
cargo test

# Benchmark
cargo bench
```

## 📦 Telepítés

### Local Development
```bash
# Development módban (változtatások azonnal látszódnak)
maturin develop
```

### Wheel-ből
```bash
# Build után
pip install target/wheels/mongolite-*.whl
```

### Editable Install
```bash
pip install -e .
```

## 🧪 Tesztelés

### Rust tesztek
```bash
cargo test
cargo test --release
```

### Python tesztek (később)
```bash
pytest tests/
```

### Manuális teszt
```bash
python example.py
```

## 🔍 Troubleshooting

### Hiba: "maturin: command not found"
```bash
# Ellenőrizd, hogy a pip bin könyvtár a PATH-ban van
echo $PATH

# Vagy telepítsd újra
pip install --user maturin
```

### Hiba: "linker 'cc' not found"
```bash
# Linux (Ubuntu/Debian)
sudo apt install build-essential

# macOS (Xcode tools)
xcode-select --install
```

### Hiba: "Python.h not found"
```bash
# Linux (Ubuntu/Debian)
sudo apt install python3-dev

# Fedora/RHEL
sudo dnf install python3-devel
```

### macOS specifikus
```bash
# Ha nem találja a Python framework-öt
export PYTHON_SYS_EXECUTABLE=/usr/local/bin/python3
maturin develop
```

### Windows specifikus

#### Hiba: "LINK : fatal error LNK1181"
```powershell
# Microsoft C++ Build Tools hiányzik
# Telepítsd: https://visualstudio.microsoft.com/visual-cpp-build-tools/
```

#### Hiba: "error: linker 'link.exe' not found"
```powershell
# Visual Studio Developer Command Prompt használata
# VAGY add hozzá a VS tools-t a PATH-hoz
# VAGY telepítsd újra a Build Tools-t
```

#### Hiba: "python3: command not found"
```powershell
# Windows-on használd a 'python' parancsot (nem 'python3')
python --version
pip --version
```

#### Virtuális környezet Windows-on
```powershell
# PowerShell
python -m venv venv
.\venv\Scripts\Activate.ps1

# Command Prompt (cmd)
python -m venv venv
venv\Scripts\activate.bat

# Ezután build
maturin develop
```

## 🌐 Platform Support

### Linux ✅
- Ubuntu 20.04+
- Debian 11+
- Fedora 35+
- Arch Linux

### macOS ✅
- macOS 11+ (Big Sur)
- Apple Silicon (M1/M2) supported
- Intel x86_64

### Windows ✅
- Windows 10/11
- MSVC toolchain required

## 📊 Build Méret

```
Debug build:   ~15 MB
Release build: ~2-3 MB (stripped)
```

## ⚙️ Build Opciók

### Cargo.toml optimalizálás
```toml
[profile.release]
opt-level = 3          # Maximum optimalizálás
lto = true            # Link-time optimization
codegen-units = 1     # Jobb optimalizálás, lassabb build
strip = true          # Debug szimbólumok eltávolítása
```

### Feature flags (később)
```bash
# Csak specifikus feature-ökkel
cargo build --features "encryption,compression"
```

## 🚢 Publikálás (később)

### PyPI
```bash
# Build minden platformra
maturin build --release

# Publikálás
maturin publish
```

### Crates.io (Rust library)
```bash
cargo publish
```

## 📝 Build Script Példák

### Linux/macOS
```bash
#!/bin/bash
# build.sh

set -e  # Exit on error

echo "🔨 Building MongoLite..."

# Tisztítás
cargo clean

# Build
maturin build --release

# Install
pip install --force-reinstall target/wheels/*.whl

# Test
python example.py

echo "✅ Build complete!"
```

### Windows (PowerShell)
```powershell
# build.ps1

Write-Host "🔨 Building MongoLite..." -ForegroundColor Green

# Tisztítás
cargo clean

# Build
maturin build --release

# Install
pip install --force-reinstall (Get-ChildItem target/wheels/*.whl)

# Test
python example.py

Write-Host "✅ Build complete!" -ForegroundColor Green
```

## 🐳 Docker Build (opcionális)

```dockerfile
FROM rust:1.70 as builder

WORKDIR /app
COPY . .

RUN pip install maturin
RUN maturin build --release

FROM python:3.11-slim
COPY --from=builder /app/target/wheels/*.whl .
RUN pip install *.whl
```

## 📚 További Források

- Rust Book: https://doc.rust-lang.org/book/
- PyO3 Guide: https://pyo3.rs/
- Maturin Docs: https://www.maturin.rs/

## ❓ Gyakori Kérdések

**Q: Mennyi ideig tart a build?**
A: Debug: ~30 sec, Release: ~2-3 perc (először)

**Q: Kell nekem Rust, ha csak használni akarom?**
A: Nem! Csak a binary wheel-t kell telepíteni (pip install)

**Q: Működik virtuális környezetben?**
A: Igen! Ajánlott is.

```bash
python -m venv venv
source venv/bin/activate  # Linux/macOS
# vagy
venv\Scripts\activate     # Windows

maturin develop
```

---

**Happy Building! 🎉**
