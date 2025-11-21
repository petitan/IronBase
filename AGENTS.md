# Repository Guidelines

## Projektstruktúra és modulok
- **Mag (`ironbase-core/src/`)**: Rust források a tároláshoz (`storage/`), lekérdezéshez (`query/`), indexekhez, WAL-hoz és tartóssághoz.
- **Python kötés (`bindings/python/src/lib.rs`)**: `pyo3`-n keresztül exportálja a Rust API-t; módosítsd, ha a felszíni API változik.
- **Példák (`examples/*.py`, `example.py`)**: MongoDB-szerű API minták, gyors füstteszteknek is jók.
- **Tesztfájlok**: Python szkriptek a gyökérben (`test_*.py`, `run_all_tests.py`), Rust unit/integration tesztek az `ironbase-core/tests` alatt.
- **Doksi és design**: Felső szintű `*.md` fájlok; frissítsd a megfelelőt, ha architektúrát vagy tartósságot érintesz.

## Build-, teszt- és fejlesztői parancsok
- Rust library: `cargo build` / `cargo build --release` a workspace gyökeréből.
- Python fejlesztői install: `maturin develop` (`pyproject.toml` vezérli).
- Csomag build: `maturin build --release` → wheel-ek a `target/wheels/` alatt.
- Rust ellenőrzések: `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test` (opcionálisan `--release`).
- Python kör: `python run_all_tests.py` futtatja az összes `test_*.py`-t. Gyors ellenőrzés: `python test_transactions.py`, stb.
- Benchmarkok (opcionális): `cargo bench` és teljesítmény szkriptek, pl. `performance_test.py`.

## Kódstílus és elnevezések
- Rust: 2021 edition; formázd `cargo fmt`-tel; clippy tisztán (`-D warnings`). Használj típusos struct/enum-okat és `Result<T>`-et `thiserror`-rel az `error.rs` alapján.
- Python: 4 szóközös behúzás; snake_case függvény/változó, PascalCase osztályok; tükrözd a MongoDB API-t (`insert_one`, `find`, `$` operátorok). Új API-khoz használj type hint-et.
- Fájlnevek: Rust modul = fájlnév (pl. `query_planner.rs`); Python példák/tesztek: `test_<téma>.py`.

## Tesztelési irányelvek
- Előny: Python integrációs tesztek végponttól végpontig, Rust tesztek motorinvariánsokra; FFI határnál mindkettőt érintsd.
- Takarítsd el az artefaktumokat (`*.mlite`, `*.wal`, `*.db`), hogy ne szivárogjon állapot; kövesd a meglévő `test_*.py` mintákat.
- Új suite esetén add a `TEST_SUITES`-hez a `run_all_tests.py`-ben; legyenek egyértelmű assert-ek, és foglald bele a crash/tartóssági lefedettséget, ha WAL-t vagy durability-t módosítasz.

## Commit- és PR-irányelvek
- Commit üzenet: rövid, felszólító, szükség esetén scope prefix-szel (`fix: ...`, `docs: ...`, `feat: ...`).
- PR: tömör összefoglaló, funkcionális változások listája, hivatkozás az érintett design doksikra, tartóssági vagy API-hatás kiemelése. Írd le a futtatott teszteket (`cargo test`, `python run_all_tests.py`, benchmarkok) és csatolj logokat hibához vagy perf állításhoz.
- Ne committeld a generált adatbázisokat vagy wheel-eket; a diff legyen fókuszált és könnyen átnézhető.
