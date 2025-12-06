# IronBase - Build and Installation Guide

[Magyar verzió / Hungarian version](BUILD.md)

## Prerequisites

### 1. Install Rust

#### Linux / macOS
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

#### Windows
1. Download the Rust installer: https://rustup.rs/
2. Run `rustup-init.exe`
3. **IMPORTANT**: The MSVC toolchain is installed automatically, but you need **Microsoft C++ Build Tools**

**Installing Microsoft C++ Build Tools:**
- Download: https://visualstudio.microsoft.com/visual-cpp-build-tools/
- Or install Visual Studio with the "Desktop development with C++" workload
- Minimum requirements:
  - MSVC v142+ (or newer)
  - Windows 10 SDK

Verify installation:
```bash
rustc --version
cargo --version
```

### 2. Install Python
```bash
# Minimum: Python 3.8
python --version
# or
python3 --version
```

### 3. Install Maturin
```bash
pip install maturin
# or
pip3 install maturin
```

## Build Process

### Development Build (Fast, Debug)
```bash
cd MongoLite

# Build and install as Python package
maturin develop

# After successful build:
python example.py
```

### Release Build (Optimized)
```bash
# Full optimization
maturin build --release

# Wheel file (platform-specific):
ls target/wheels/
# Linux:   ironbase-0.2.0-cp38-abi3-linux_x86_64.whl
# Windows: ironbase-0.2.0-cp38-abi3-win_amd64.whl
# macOS:   ironbase-0.2.0-cp38-abi3-macosx_11_0_universal2.whl
```

### Rust-Only Build (Without Python)
```bash
# Library build
cargo build --release

# Run tests
cargo test

# Run benchmarks
cargo bench
```

## Installation

### Local Development
```bash
# Development mode (changes visible immediately)
maturin develop
```

### From Wheel
```bash
# After build
pip install target/wheels/ironbase-*.whl
```

### Editable Install
```bash
pip install -e .
```

## Testing

### Rust Tests
```bash
cargo test
cargo test --release
```

### Python Tests
```bash
pytest tests/
```

### Manual Test
```bash
python example.py
```

## Troubleshooting

### Error: "maturin: command not found"
```bash
# Check if pip bin directory is in PATH
echo $PATH

# Or reinstall
pip install --user maturin
```

### Error: "linker 'cc' not found"
```bash
# Linux (Ubuntu/Debian)
sudo apt install build-essential

# macOS (Xcode tools)
xcode-select --install
```

### Error: "Python.h not found"
```bash
# Linux (Ubuntu/Debian)
sudo apt install python3-dev

# Fedora/RHEL
sudo dnf install python3-devel
```

### macOS Specific
```bash
# If Python framework not found
export PYTHON_SYS_EXECUTABLE=/usr/local/bin/python3
maturin develop
```

### Windows Specific

#### Error: "LINK : fatal error LNK1181"
```powershell
# Microsoft C++ Build Tools is missing
# Install: https://visualstudio.microsoft.com/visual-cpp-build-tools/
```

#### Error: "error: linker 'link.exe' not found"
```powershell
# Use Visual Studio Developer Command Prompt
# OR add VS tools to PATH
# OR reinstall Build Tools
```

#### Error: "python3: command not found"
```powershell
# On Windows use 'python' command (not 'python3')
python --version
pip --version
```

#### Virtual Environment on Windows
```powershell
# PowerShell
python -m venv venv
.\venv\Scripts\Activate.ps1

# Command Prompt (cmd)
python -m venv venv
venv\Scripts\activate.bat

# Then build
maturin develop
```

## Platform Support

### Linux
- Ubuntu 20.04+
- Debian 11+
- Fedora 35+
- Arch Linux

### macOS
- macOS 11+ (Big Sur)
- Apple Silicon (M1/M2) supported
- Intel x86_64

### Windows
- Windows 10/11
- MSVC toolchain required

## Build Size

```
Debug build:   ~15 MB
Release build: ~2-3 MB (stripped)
```

## Build Options

### Cargo.toml Optimization
```toml
[profile.release]
opt-level = 3          # Maximum optimization
lto = true            # Link-time optimization
codegen-units = 1     # Better optimization, slower build
strip = true          # Remove debug symbols
```

### Feature Flags (Future)
```bash
# Build with specific features only
cargo build --features "encryption,compression"
```

## Publishing

### PyPI
```bash
# Build for all platforms
maturin build --release

# Publish
maturin publish
```

### Crates.io (Rust Library)
```bash
cargo publish
```

## Build Script Examples

### Linux/macOS
```bash
#!/bin/bash
# build.sh

set -e  # Exit on error

echo "Building IronBase..."

# Clean
cargo clean

# Build
maturin build --release

# Install
pip install --force-reinstall target/wheels/*.whl

# Test
python example.py

echo "Build complete!"
```

### Windows (PowerShell)
```powershell
# build.ps1

Write-Host "Building IronBase..." -ForegroundColor Green

# Clean
cargo clean

# Build
maturin build --release

# Install
pip install --force-reinstall (Get-ChildItem target/wheels/*.whl)

# Test
python example.py

Write-Host "Build complete!" -ForegroundColor Green
```

## Docker Build (Optional)

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

## Additional Resources

- Rust Book: https://doc.rust-lang.org/book/
- PyO3 Guide: https://pyo3.rs/
- Maturin Docs: https://www.maturin.rs/

## FAQ

**Q: How long does the build take?**
A: Debug: ~30 sec, Release: ~2-3 min (first time)

**Q: Do I need Rust to use IronBase?**
A: No! Just install the binary wheel (`pip install ironbase`)

**Q: Does it work in virtual environments?**
A: Yes! It's recommended.

```bash
python -m venv venv
source venv/bin/activate  # Linux/macOS
# or
venv\Scripts\activate     # Windows

maturin develop
```

---

**Happy Building!**
