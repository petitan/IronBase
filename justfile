# Use Bash for shell commands
set shell := ["bash", "-c"]

default:
    @just --list

fmt:
    cargo fmt
    (cd mcp-server && cargo fmt)

lint:
    cargo clippy --all-targets --all-features
    (cd mcp-server && cargo clippy --all-targets --all-features)

test-core:
    cargo test -p ironbase-core

test-mcp:
    (cd mcp-server && cargo test)

test-python-auto:
    source venv/bin/activate && python3 mcp-server/test_python_auto_commit.py

seed-test-doc:
    source venv/bin/activate && python3 mcp-server/seed_test_doc.py

run-dev-checks:
    ./scripts/run_dev_checks.sh
