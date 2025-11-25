#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

run_fmt() {
    if cargo fmt --version >/dev/null 2>&1; then
        cargo fmt "$@"
    else
        echo "⚠️  cargo fmt not available (install rustfmt) – skipping fmt"
    fi
}

run_clippy() {
    if cargo clippy --version >/dev/null 2>&1; then
        cargo clippy "$@"
    else
        echo "⚠️  cargo clippy not available – skipping clippy"
    fi
}

echo "▶ Running Rust fmt/clippy/tests (workspace root)"
(
    cd "$PROJECT_ROOT"
    run_fmt
    run_clippy --all-targets --all-features
    cargo test -p ironbase-core
)

echo "▶ Running Rust fmt/clippy/tests (mcp-server)"
(
    cd "$PROJECT_ROOT/mcp-server"
    run_fmt
    run_clippy --all-targets --all-features
    cargo test
)

if [[ -d "$PROJECT_ROOT/venv" ]]; then
    echo "▶ Running Python smoke tests (auto-commit suite)"
    source "$PROJECT_ROOT/venv/bin/activate"
    python3 "$PROJECT_ROOT/test_python_auto_commit.py"
else
    echo "⚠️  Python virtual environment not found at $PROJECT_ROOT/venv – skipping Python smoke tests"
fi

echo "✅ Dev checks finished"
