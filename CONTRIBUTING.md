# Contributing to IronBase

Thank you for your interest in contributing to IronBase! This document provides guidelines and instructions for contributing.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Environment](#development-environment)
- [Making Changes](#making-changes)
- [Testing](#testing)
- [Pull Request Process](#pull-request-process)
- [Code Style](#code-style)
- [Commit Messages](#commit-messages)

## Code of Conduct

This project adheres to the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code.

## Getting Started

1. Fork the repository on GitHub
2. Clone your fork locally:
   ```bash
   git clone https://github.com/YOUR_USERNAME/IronBase.git
   cd IronBase
   ```
3. Add the upstream repository:
   ```bash
   git remote add upstream https://github.com/petitan/IronBase.git
   ```

## Development Environment

### Prerequisites

- **Rust**: 1.75+ (install via [rustup](https://rustup.rs/))
- **Python**: 3.8+ (for Python bindings)
- **Maturin**: For building Python wheels
- **.NET SDK**: 8.0+ (for C# bindings)
- **Just**: Task runner (optional but recommended)

### Setup

```bash
# Install Rust tools
rustup update stable
rustup component add rustfmt clippy

# Install Python dependencies
pip install maturin pytest

# Build the project
cargo build

# Build Python bindings (development mode)
maturin develop

# Build .NET bindings
cd IronBase.NET
dotnet build
```

### Using Just

If you have [just](https://github.com/casey/just) installed:

```bash
just fmt        # Format code
just lint       # Run clippy
just test-core  # Run Rust tests
just run-dev-checks  # Run all checks
```

## Making Changes

1. Create a new branch for your changes:
   ```bash
   git checkout -b feature/your-feature-name
   # or
   git checkout -b fix/your-bug-fix
   ```

2. Make your changes, following the [code style guidelines](#code-style)

3. Add or update tests as needed

4. Run the test suite to ensure everything passes

5. Commit your changes following the [commit message guidelines](#commit-messages)

## Testing

### Rust Tests

```bash
# Run all Rust tests
cargo test --workspace

# Run core library tests only
cargo test -p ironbase-core

# Run specific test
cargo test test_name

# Run with output
cargo test -- --nocapture
```

### Python Tests

```bash
# Build development version first
maturin develop

# Run Python tests
python -m pytest tests/
# or
python run_all_tests.py
```

### .NET Tests

```bash
cd IronBase.NET
dotnet test
```

## Pull Request Process

1. **Update your branch** with the latest upstream changes:
   ```bash
   git fetch upstream
   git rebase upstream/master
   ```

2. **Run all checks** before submitting:
   ```bash
   just run-dev-checks
   # or manually:
   cargo fmt --check
   cargo clippy --all-targets --all-features
   cargo test --workspace
   ```

3. **Create a Pull Request** with:
   - A clear title describing the change
   - A description explaining what and why
   - Reference to any related issues

4. **Address review feedback** promptly

5. **Squash commits** if requested before merge

## Code Style

### Rust

- Follow the official [Rust Style Guide](https://doc.rust-lang.org/style-guide/)
- Use `cargo fmt` to format code
- Address all `cargo clippy` warnings
- Add documentation comments for public APIs
- Use meaningful variable and function names

### Python

- Follow [PEP 8](https://pep8.org/)
- Use type hints where appropriate
- Add docstrings for public functions

### C#

- Follow [Microsoft C# Coding Conventions](https://docs.microsoft.com/en-us/dotnet/csharp/fundamentals/coding-style/coding-conventions)
- Use XML documentation comments for public APIs

## Commit Messages

We follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

### Types

- `feat`: A new feature
- `fix`: A bug fix
- `docs`: Documentation changes
- `style`: Code style changes (formatting, etc.)
- `refactor`: Code refactoring without feature change
- `perf`: Performance improvements
- `test`: Adding or updating tests
- `chore`: Maintenance tasks
- `ci`: CI/CD changes

### Examples

```
feat(query): add $regex operator support

fix(storage): prevent data corruption on concurrent writes

docs(readme): update installation instructions

test(collection): add integration tests for update operators
```

## Reporting Issues

When reporting bugs, please include:

1. IronBase version
2. Operating system and version
3. Rust/Python/.NET version
4. Minimal reproducible example
5. Expected vs actual behavior
6. Error messages or stack traces

## Questions?

Feel free to open a [Discussion](https://github.com/petitan/IronBase/discussions) for questions or ideas.

---

Thank you for contributing to IronBase!
