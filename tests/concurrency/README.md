# Concurrency Anomaly Tests

Shell-based tests for MCP server concurrency behavior. Requires MCP server running on `http://127.0.0.1:8080`.

## Prerequisites

```bash
cd mcp-server && cargo build --release
./target/release/mcp-ironbase-server &
```

## Tests

| Test | File | What it tests |
|------|------|---------------|
| Lost Update | `lost_update_dirty_read.sh` | $inc atomicity under concurrent access |
| Dirty Read | `lost_update_dirty_read.sh` | Reading uncommitted data |
| Non-Repeatable Read | `non_repeatable_read_test.sh` | Same query, different results |
| Phantom Read | `phantom_read_test.sh` | Row count changes between queries |
| Read Skew | `read_write_skew_test.sh` | Inconsistent reads across documents |
| Write Skew | `read_write_skew_test.sh` | Constraint violations via concurrent writes |
| Double Spending | `double_spending_test.sh` | Financial double-spend attack |
| ABA Problem | `aba_problem_test.sh` | Undetected A->B->A value changes |

## Run All

```bash
./lost_update_dirty_read.sh
./non_repeatable_read_test.sh
./phantom_read_test.sh
./read_write_skew_test.sh
./double_spending_test.sh
./aba_problem_test.sh
```

## Expected Results (IronBase READ COMMITTED)

| Anomaly | Status | Notes |
|---------|--------|-------|
| Lost Update | PROTECTED | Atomic update path |
| Dirty Read | PROTECTED | Write lock isolation |
| Non-Repeatable Read | ~16% | Expected at this isolation level |
| Phantom Read | 0% | Expected (no explicit transactions) |
| Read Skew | ~30% | Expected at this isolation level |
| Write Skew | Possible | Use conditional updates |
| Double Spending | PROTECTED | With conditional update pattern |
| ABA Problem | PROTECTED | With version field pattern |
