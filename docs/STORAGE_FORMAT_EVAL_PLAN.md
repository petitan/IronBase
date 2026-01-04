# Storage Format Evaluation Plan (JSON vs Alternatives)

## Goals
- Decide whether switching from JSON storage is justified for 100MB+ collections.
- Quantify impact on IO, CPU, memory, and cache efficiency.
- Minimize risk by using real workloads and repeatable measurements.

## Scope
- Current JSON storage path (baseline).
- Candidate format: CBOR + optional field dictionary.
- Candidate format: custom binary (field-id + varint + optional zstd).
- MessagePack excluded unless it wins clearly in metrics.

## Workloads to Measure
1) **Bulk Insert**  
   - Import 100k–500k documents.
   - Metrics: write throughput (docs/s), bytes written, WAL size.
2) **Read + Projection**  
   - Query with projection (few fields).
   - Metrics: latency, bytes read, CPU time in decode.
3) **Aggregation**  
   - $match + $group + $count.
   - Metrics: total time, doc decode time, memory peak.
4) **Index Build**  
   - Create index on high-cardinality field.
   - Metrics: build time, peak memory, temp IO.

## Metrics to Capture
- Wall time, CPU time (user/sys).
- Bytes read/written (OS counters or file size deltas).
- Allocations / peak RSS.
- Deserialize/serialize time (instrumentation).
- Cache hit rate (if available).

## Success Criteria
- >= 20% end-to-end improvement on at least 2 workloads.
- <= 5% regression on any workload.
- No increase in corruption risk or recovery time.

## Experiment Design
1) Use the same dataset snapshot for all runs.
2) Run each workload 3x, report median + p95.
3) Clear OS cache between runs if possible (or at least note warm vs cold).
4) Track CPU/memory per run.

## Implementation Steps (Minimal Risk)
1) Add instrumentation to JSON path for decode/encode timings.
2) Implement CBOR storage (no dictionary) behind a feature flag.
3) Compare JSON vs CBOR on workloads.
4) If CBOR gains are marginal, stop.
5) If CBOR gains are promising, prototype field dictionary.
6) Only then consider full custom binary.

## Deliverables
- Benchmark report (tables + charts).
- Recommendation decision (keep JSON / move to CBOR / custom binary).
- Migration and fallback plan (if switching).
