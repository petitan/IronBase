# Query Optimizer - Részletes Implementációs Terv

## Tartalomjegyzék
1. [Áttekintés](#áttekintés)
2. [Query Execution Plan](#query-execution-plan)
3. [Index Selection Stratégia](#index-selection-stratégia)
4. [Cost-Based Optimization](#cost-based-optimization)
5. [Query Rewrite Szabályok](#query-rewrite-szabályok)
6. [Algoritmusok és Pszeudokód](#algoritmusok-és-pszeudokód)
7. [Implementációs Példák](#implementációs-példák)
8. [Teljesítmény Mérés](#teljesítmény-mérés)

---

## Áttekintés

A Query Optimizer célja a leghatékonyabb végrehajtási terv kiválasztása egy query-hez. Döntéseket hoz:
- Index használat vs. full collection scan
- Melyik indexet használja (ha több van)
- Operátorok végrehajtási sorrendje
- Filter pushdown optimalizálás

### Célok MVP-ben
- ✅ Egyszerű index selection
- ✅ Full scan vs. index scan döntés
- ✅ Range query optimalizálás
- ✅ Execution plan generálás

### Későbbi (v0.4.0+)
- Statistics-based optimization
- Multi-index strategies
- Join optimization (ha lesz)
- Query caching

---

## Query Execution Plan

### ExecutionPlan Struktúra

```rust
#[derive(Debug, Clone)]
pub struct ExecutionPlan {
    /// Plan típusa
    pub plan_type: PlanType,

    /// Becsült költség
    pub estimated_cost: f64,

    /// Becsült eredmény méret
    pub estimated_rows: usize,

    /// Lépések
    pub steps: Vec<ExecutionStep>,
}

#[derive(Debug, Clone)]
pub enum PlanType {
    /// Teljes collection scan
    CollectionScan,

    /// Index scan
    IndexScan {
        index_name: String,
        index_field: String,
    },

    /// Index scan + filter
    IndexScanWithFilter {
        index_name: String,
        index_field: String,
        additional_filters: Vec<Filter>,
    },
}

#[derive(Debug, Clone)]
pub enum ExecutionStep {
    /// Collection scan lépés
    ScanCollection {
        estimated_docs: usize,
    },

    /// Index lookup
    IndexLookup {
        index_name: String,
        key_range: KeyRange,
        estimated_keys: usize,
    },

    /// Document fetch (index után)
    FetchDocuments {
        offsets: Vec<u64>,
    },

    /// Filter alkalmazása
    ApplyFilter {
        filter: Filter,
        estimated_selectivity: f64,
    },

    /// Sort
    Sort {
        field: String,
        direction: SortDirection,
    },

    /// Limit/Skip
    LimitSkip {
        limit: Option<usize>,
        skip: Option<usize>,
    },
}
```

---

## Index Selection Stratégia

### Heurisztikus Szabályok (MVP)

```
FUNCTION select_best_index(query, available_indexes) -> Option<String>:
    candidates = []

    // 1. _id egyenlőség - legmagasabb prioritás
    IF query.has_field_equality("_id"):
        RETURN Some("_id_")
    END IF

    // 2. Unique index egyenlőség
    FOR index IN available_indexes:
        IF index.unique AND query.has_field_equality(index.field):
            candidates.push((index.name, 1000))  // Highest score
        END IF
    END FOR

    // 3. Egyenlőség bármely indexen
    FOR index IN available_indexes:
        IF query.has_field_equality(index.field):
            candidates.push((index.name, 500))
        END IF
    END FOR

    // 4. Range query indexen
    FOR index IN available_indexes:
        IF query.has_field_range(index.field):
            // Szelektivitás becslés
            selectivity = estimate_range_selectivity(query, index.field)
            score = 100 / selectivity  // Kisebb selectivity = jobb
            candidates.push((index.name, score))
        END IF
    END FOR

    // 5. Létezés check
    FOR index IN available_indexes:
        IF query.has_field_exists(index.field):
            candidates.push((index.name, 10))
        END IF
    END FOR

    // Legjobb kiválasztása
    IF candidates.empty():
        RETURN None  // Full scan
    END IF

    candidates.sort_by_score_desc()
    RETURN Some(candidates[0].name)
END FUNCTION
```

### Döntési Fa

```
Query → Index Selection Decision Tree

1. _id = value?
   ├─ YES → Use _id index ✅
   └─ NO  → Continue

2. Unique indexed field = value?
   ├─ YES → Use unique index ✅
   └─ NO  → Continue

3. Any indexed field = value?
   ├─ YES → Use that index ✅
   └─ NO  → Continue

4. Range operator on indexed field?
   ├─ YES → Calculate selectivity
   │        ├─ Selectivity < 0.3 → Use index ✅
   │        └─ Selectivity ≥ 0.3 → Full scan (cheaper)
   └─ NO  → Continue

5. $exists on indexed field?
   ├─ YES → Sparse index check
   │        ├─ Sparse → Use index ✅
   │        └─ Not sparse → Full scan
   └─ NO  → Full scan
```

---

## Cost-Based Optimization

### Cost Model (Egyszerűsített MVP)

```rust
#[derive(Debug)]
pub struct CostModel {
    /// Document read költsége (ms)
    pub doc_read_cost: f64,

    /// Index lookup költsége (ms)
    pub index_lookup_cost: f64,

    /// Deserialize költsége (ms per doc)
    pub deserialize_cost: f64,

    /// Filter költsége (ms per doc)
    pub filter_cost: f64,
}

impl Default for CostModel {
    fn default() -> Self {
        CostModel {
            doc_read_cost: 0.01,      // 10 μs per doc
            index_lookup_cost: 0.001, // 1 μs per index node
            deserialize_cost: 0.005,  // 5 μs per doc
            filter_cost: 0.002,       // 2 μs per filter check
        }
    }
}

impl CostModel {
    /// Full scan költség becslés
    pub fn estimate_full_scan_cost(&self, num_docs: usize, num_filters: usize) -> f64 {
        let read_cost = num_docs as f64 * self.doc_read_cost;
        let deserialize_cost = num_docs as f64 * self.deserialize_cost;
        let filter_cost = num_docs as f64 * num_filters as f64 * self.filter_cost;

        read_cost + deserialize_cost + filter_cost
    }

    /// Index scan költség becslés
    pub fn estimate_index_scan_cost(
        &self,
        tree_height: usize,
        num_matches: usize,
        num_additional_filters: usize,
    ) -> f64 {
        // Index traversal
        let index_cost = tree_height as f64 * self.index_lookup_cost;

        // Document fetch
        let fetch_cost = num_matches as f64 * self.doc_read_cost;
        let deserialize_cost = num_matches as f64 * self.deserialize_cost;

        // Additional filtering
        let filter_cost = num_matches as f64 * num_additional_filters as f64 * self.filter_cost;

        index_cost + fetch_cost + deserialize_cost + filter_cost
    }
}
```

### Szelektivitás Becslés

```rust
pub fn estimate_selectivity(query: &Query, field: &str, stats: &CollectionStats) -> f64 {
    if let Some(filter) = query.get_filter_for_field(field) {
        match filter {
            // Egyenlőség: 1 / distinct_values
            Filter::Eq(_) => {
                1.0 / stats.distinct_values.get(field).unwrap_or(&stats.total_docs) as f64
            }

            // Range: heurisztika alapján
            Filter::Gt(_) | Filter::Gte(_) | Filter::Lt(_) | Filter::Lte(_) => {
                // Alapértelmezés: 30% (konzervatív becslés)
                // Későbbi: histogram alapú becslés
                0.3
            }

            // $in operator: length / distinct_values
            Filter::In(values) => {
                let distinct = stats.distinct_values.get(field).unwrap_or(&stats.total_docs);
                (values.len() as f64 / *distinct as f64).min(1.0)
            }

            // $exists: sparse index stats
            Filter::Exists(true) => {
                stats.field_presence.get(field).unwrap_or(&stats.total_docs) as f64
                    / stats.total_docs as f64
            }

            _ => 0.5,  // Alapértelmezett
        }
    } else {
        1.0  // Nincs filter
    }
}
```

---

## Query Rewrite Szabályok

### Optimalizációs Szabályok

#### 1. Constant Folding

```
BEFORE: {age: {$gt: 10 + 20}}
AFTER:  {age: {$gt: 30}}
```

#### 2. Range Normalization

```
BEFORE: {age: {$gte: 20, $lt: 30, $gt: 25}}
AFTER:  {age: {$gt: 25, $lt: 30}}  // $gt: 25 erősebb mint $gte: 20
```

#### 3. Redundant Filter Removal

```
BEFORE: {$and: [{age: {$gt: 20}}, {age: {$gt: 20}}]}
AFTER:  {age: {$gt: 20}}
```

#### 4. DNF (Disjunctive Normal Form) Conversion

```
BEFORE: {$or: [{$and: [A, B]}, {$and: [A, C]}]}
AFTER:  {$and: [A, {$or: [B, C]}]}  // A közös faktor
```

#### 5. Index-friendly Rewrite

```
BEFORE: {$or: [{age: 25}, {age: 30}, {age: 35}]}
AFTER:  {age: {$in: [25, 30, 35]}}  // Index használhat $in-t
```

---

### Rewrite Engine Pszeudokód

```
FUNCTION rewrite_query(query) -> Query:
    query = constant_folding(query)
    query = normalize_ranges(query)
    query = remove_redundant_filters(query)
    query = convert_or_to_in(query)
    query = simplify_boolean_logic(query)
    RETURN query
END FUNCTION

FUNCTION normalize_ranges(query) -> Query:
    FOR field, filters IN query.fields:
        gt_values = []
        gte_values = []
        lt_values = []
        lte_values = []

        FOR filter IN filters:
            MATCH filter:
                CASE Gt(val): gt_values.push(val)
                CASE Gte(val): gte_values.push(val)
                CASE Lt(val): lt_values.push(val)
                CASE Lte(val): lte_values.push(val)
            END MATCH
        END FOR

        // Strongest lower bound
        lower_bound = None
        IF NOT gt_values.empty():
            lower_bound = Some(Gt(max(gt_values)))
        ELSE IF NOT gte_values.empty():
            lower_bound = Some(Gte(max(gte_values)))
        END IF

        // Strongest upper bound
        upper_bound = None
        IF NOT lt_values.empty():
            upper_bound = Some(Lt(min(lt_values)))
        ELSE IF NOT lte_values.empty():
            upper_bound = Some(Lte(min(lte_values)))
        END IF

        // Replace with normalized
        query.set_field_filters(field, [lower_bound, upper_bound].compact())
    END FOR

    RETURN query
END FUNCTION
```

---

## Algoritmusok és Pszeudokód

### Query Optimization Pipeline

```
FUNCTION optimize_query(query, collection_stats, available_indexes) -> ExecutionPlan:
    // 1. Query rewrite
    rewritten_query = rewrite_query(query)

    // 2. Index selection
    selected_index = select_best_index(rewritten_query, available_indexes)

    // 3. Plan generation
    IF selected_index.is_some():
        plan = generate_index_plan(rewritten_query, selected_index.unwrap())
    ELSE:
        plan = generate_full_scan_plan(rewritten_query)
    END IF

    // 4. Cost estimation
    plan.estimated_cost = estimate_plan_cost(plan, collection_stats)

    // 5. Alternative plans (későbbi)
    // alternatives = generate_alternative_plans(rewritten_query)
    // best_plan = choose_cheapest(plan, alternatives)

    RETURN plan
END FUNCTION
```

### Full Scan Plan Generation

```
FUNCTION generate_full_scan_plan(query) -> ExecutionPlan:
    plan = ExecutionPlan::new()
    plan.plan_type = PlanType::CollectionScan

    // Step 1: Scan all documents
    plan.steps.push(ExecutionStep::ScanCollection {
        estimated_docs: collection_stats.total_docs
    })

    // Step 2: Apply filters
    FOR filter IN query.filters:
        selectivity = estimate_filter_selectivity(filter)
        plan.steps.push(ExecutionStep::ApplyFilter {
            filter: filter,
            estimated_selectivity: selectivity
        })
    END FOR

    // Step 3: Sort (if needed)
    IF query.has_sort():
        plan.steps.push(ExecutionStep::Sort {
            field: query.sort_field,
            direction: query.sort_direction
        })
    END IF

    // Step 4: Limit/Skip
    IF query.has_limit() OR query.has_skip():
        plan.steps.push(ExecutionStep::LimitSkip {
            limit: query.limit,
            skip: query.skip
        })
    END IF

    RETURN plan
END FUNCTION
```

### Index Scan Plan Generation

```
FUNCTION generate_index_plan(query, index_name) -> ExecutionPlan:
    plan = ExecutionPlan::new()
    index = get_index(index_name)

    plan.plan_type = PlanType::IndexScan {
        index_name: index_name,
        index_field: index.field
    }

    // Step 1: Index lookup
    key_range = extract_key_range(query, index.field)
    estimated_matches = estimate_index_matches(key_range, index)

    plan.steps.push(ExecutionStep::IndexLookup {
        index_name: index_name,
        key_range: key_range,
        estimated_keys: estimated_matches
    })

    // Step 2: Fetch documents
    plan.steps.push(ExecutionStep::FetchDocuments {
        offsets: vec![]  // Filled at runtime
    })

    // Step 3: Additional filters (non-index fields)
    additional_filters = query.filters_not_on_field(index.field)
    FOR filter IN additional_filters:
        selectivity = estimate_filter_selectivity(filter)
        plan.steps.push(ExecutionStep::ApplyFilter {
            filter: filter,
            estimated_selectivity: selectivity
        })
    END FOR

    // Step 4: Sort (if not already sorted by index)
    IF query.has_sort() AND query.sort_field != index.field:
        plan.steps.push(ExecutionStep::Sort { ... })
    END IF

    // Step 5: Limit/Skip
    IF query.has_limit() OR query.has_skip():
        plan.steps.push(ExecutionStep::LimitSkip { ... })
    END IF

    RETURN plan
END FUNCTION
```

---

## Implementációs Példák

### Query Optimizer Rust Implementáció

```rust
// src/query_optimizer.rs
pub struct QueryOptimizer {
    cost_model: CostModel,
}

impl QueryOptimizer {
    pub fn new() -> Self {
        QueryOptimizer {
            cost_model: CostModel::default(),
        }
    }

    pub fn optimize(
        &self,
        query: &Query,
        stats: &CollectionStats,
        indexes: &HashMap<String, IndexMeta>,
    ) -> Result<ExecutionPlan> {
        // 1. Rewrite
        let rewritten = self.rewrite_query(query);

        // 2. Index selection
        let selected_index = self.select_index(&rewritten, indexes, stats);

        // 3. Generate plan
        let plan = match selected_index {
            Some(index_name) => {
                self.generate_index_plan(&rewritten, &index_name, indexes, stats)?
            }
            None => {
                self.generate_full_scan_plan(&rewritten, stats)?
            }
        };

        Ok(plan)
    }

    fn select_index(
        &self,
        query: &Query,
        indexes: &HashMap<String, IndexMeta>,
        stats: &CollectionStats,
    ) -> Option<String> {
        let mut candidates: Vec<(String, f64)> = Vec::new();

        // _id equality
        if query.has_equality_on("_id") {
            return Some("_id_".to_string());
        }

        // Score each index
        for (name, index) in indexes.iter() {
            if let Some(score) = self.score_index(query, index, stats) {
                candidates.push((name.clone(), score));
            }
        }

        if candidates.is_empty() {
            return None;
        }

        // Sort by score descending
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        Some(candidates[0].0.clone())
    }

    fn score_index(
        &self,
        query: &Query,
        index: &IndexMeta,
        stats: &CollectionStats,
    ) -> Option<f64> {
        let field = &index.field;

        // Equality on unique index
        if index.unique && query.has_equality_on(field) {
            return Some(1000.0);
        }

        // Equality on regular index
        if query.has_equality_on(field) {
            return Some(500.0);
        }

        // Range query
        if query.has_range_on(field) {
            let selectivity = self.estimate_selectivity(query, field, stats);

            // Only use index if selective enough
            if selectivity < 0.3 {
                return Some(100.0 / selectivity);
            } else {
                return None;  // Full scan cheaper
            }
        }

        // Exists check on sparse index
        if index.sparse && query.has_exists_on(field) {
            return Some(10.0);
        }

        None
    }

    fn estimate_selectivity(
        &self,
        query: &Query,
        field: &str,
        stats: &CollectionStats,
    ) -> f64 {
        // Simplified selectivity estimation
        if query.has_equality_on(field) {
            1.0 / stats.total_docs as f64
        } else if query.has_range_on(field) {
            0.3  // Conservative estimate
        } else {
            1.0
        }
    }
}
```

---

### Execution Plan Pretty Print

```rust
impl ExecutionPlan {
    pub fn explain(&self) -> String {
        let mut output = String::new();

        output.push_str(&format!("Plan Type: {:?}\n", self.plan_type));
        output.push_str(&format!("Estimated Cost: {:.2}ms\n", self.estimated_cost));
        output.push_str(&format!("Estimated Rows: {}\n\n", self.estimated_rows));
        output.push_str("Steps:\n");

        for (i, step) in self.steps.iter().enumerate() {
            output.push_str(&format!("  {}. {:?}\n", i + 1, step));
        }

        output
    }
}
```

**Példa output:**
```
Plan Type: IndexScan { index_name: "age_1", index_field: "age" }
Estimated Cost: 2.50ms
Estimated Rows: 150

Steps:
  1. IndexLookup { index_name: "age_1", key_range: Range(30, 40), estimated_keys: 150 }
  2. FetchDocuments { offsets: [] }
  3. ApplyFilter { filter: Eq("city", "Budapest"), estimated_selectivity: 0.2 }
  4. LimitSkip { limit: Some(10), skip: None }
```

---

## Teljesítmény Mérés

### Benchmarking

```rust
#[cfg(test)]
mod benchmarks {
    use criterion::{black_box, criterion_group, criterion_main, Criterion};

    fn benchmark_query_optimizer(c: &mut Criterion) {
        let optimizer = QueryOptimizer::new();
        let query = Query::parse(json!({"age": {"$gt": 30}})).unwrap();
        let stats = setup_test_stats();
        let indexes = setup_test_indexes();

        c.bench_function("optimize_simple_query", |b| {
            b.iter(|| {
                optimizer.optimize(
                    black_box(&query),
                    black_box(&stats),
                    black_box(&indexes),
                )
            })
        });
    }

    criterion_group!(benches, benchmark_query_optimizer);
    criterion_main!(benches);
}
```

### Query Performance Comparison

```python
# Python benchmark
import time
from mongolite import MongoLite

db = MongoLite("benchmark.mlite")
collection = db.collection("users")

# Insert test data
for i in range(10000):
    collection.insert_one({"age": i % 100, "city": f"City{i % 10}"})

# Without index
start = time.time()
results = collection.find({"age": {"$gt": 50}})
no_index_time = time.time() - start
print(f"No index: {no_index_time*1000:.2f}ms")

# With index
collection.create_index("age")
start = time.time()
results = collection.find({"age": {"$gt": 50}})
with_index_time = time.time() - start
print(f"With index: {with_index_time*1000:.2f}ms")

print(f"Speedup: {no_index_time / with_index_time:.2f}x")
```

---

## Roadmap

### MVP (v0.3.0) - 1 hét
- ✅ Egyszerű index selection (heurisztika)
- ✅ Full scan vs. index scan döntés
- ✅ ExecutionPlan struktúra
- ✅ Alapvető cost estimation

### v0.3.5 - 1-2 hét
- ✅ Query rewrite szabályok
- ✅ Range normalization
- ✅ Redundancy removal
- ✅ Explain API (plan visualization)

### v0.4.0 - 2 hét
- ✅ Statistics collection
- ✅ Histogram-based selectivity
- ✅ Multi-index strategies
- ✅ Query plan caching

### v1.0.0 - Later
- ✅ Adaptive query optimization
- ✅ Query feedback loop
- ✅ Machine learning hints
- ✅ Advanced cost model

---

## Összefoglalás

### Kulcs Döntések

1. **Heuristic-based optimization (MVP)**
   - Egyszerű szabályok
   - Gyors döntéshozatal
   - Nincs statisztika szükséges

2. **Simple cost model**
   - Constant költség értékek
   - Linear becslés
   - Későbbi histogram support

3. **Index-first stratégia**
   - Mindig index ha van
   - Szelektivitás check
   - Fallback full scan

4. **Explain API**
   - Query plan láthatóság
   - Debug support
   - Performance tuning

### Sikerkritériumok

- ✅ Index használat automatikus
- ✅ < 1ms optimizer overhead
- ✅ 5-10x speedup indexed query-ken
- ✅ Helyes plan selection 90%+
- ✅ Explain output hasznos

---

**Következő:** `ALGORITHMS.md` - Összefoglaló algoritmusok dokumentum
