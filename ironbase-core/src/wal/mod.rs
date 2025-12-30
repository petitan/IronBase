//! # WAL (Write-Ahead Log) Modul
//!
//! ## Cél és ACID Szerep
//!
//! A WAL biztosítja az ACID tranzakciók **Durability (D)** tulajdonságát.
//! Minden írási művelet ELŐBB a WAL-ba kerül, és CSAK UTÁNA a tényleges adatfájlba.
//! Ez garantálja, hogy crash esetén az adatok helyreállíthatók.
//!
//! ## Architektúra
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                      WRITE PATH                                 │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                                                                 │
//! │   Transaction          WAL               Storage                │
//! │   ┌─────────┐    ┌─────────────┐    ┌─────────────┐            │
//! │   │ BEGIN   │───▶│ WAL::append │───▶│             │            │
//! │   │ INSERT  │───▶│ WAL::append │    │             │            │
//! │   │ COMMIT  │───▶│ WAL::append │    │             │            │
//! │   └─────────┘    │ WAL::flush  │    │             │            │
//! │                  │   (fsync)   │    │             │            │
//! │                  └──────┬──────┘    │             │            │
//! │                         │           │             │            │
//! │                         ▼           │             │            │
//! │              ┌──────────────────┐   │             │            │
//! │              │ COMMIT POINT     │   │             │            │
//! │              │ (data is durable)│   │             │            │
//! │              └────────┬─────────┘   │             │            │
//! │                       │             │             │            │
//! │                       ▼             │             │            │
//! │              Storage::apply ────────▶ flush_meta  │            │
//! │                                     │   (fsync)   │            │
//! │                                     └──────┬──────┘            │
//! │                                            │                   │
//! │                                            ▼                   │
//! │                                     WAL::clear                 │
//! │                                                                 │
//! └─────────────────────────────────────────────────────────────────┘
//!
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                     RECOVERY PATH                               │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                                                                 │
//! │   Startup              WAL                  Storage             │
//! │   ┌─────────┐    ┌─────────────┐    ┌─────────────────┐        │
//! │   │ open()  │───▶│ WAL exists? │    │                 │        │
//! │   └─────────┘    └──────┬──────┘    │                 │        │
//! │                         │           │                 │        │
//! │                    ┌────┴────┐      │                 │        │
//! │                    ▼         ▼      │                 │        │
//! │                  [YES]     [NO]     │                 │        │
//! │                    │         │      │                 │        │
//! │                    ▼         ▼      │                 │        │
//! │           WALEntryIterator  skip    │                 │        │
//! │                    │                │                 │        │
//! │                    ▼                │                 │        │
//! │           TransactionGrouper        │                 │        │
//! │           (csak COMMIT-olt TX)      │                 │        │
//! │                    │                │                 │        │
//! │                    ▼                │                 │        │
//! │           Replay operations ────────▶ apply changes  │        │
//! │                                     │                 │        │
//! │                                     ▼                 │        │
//! │                              WAL::clear              │        │
//! │                                                       │        │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## WAL Entry Bináris Formátum
//!
//! ```text
//! ┌────────────────────────────────────────────────────────┐
//! │              WAL ENTRY STRUCTURE                       │
//! ├────────────────────────────────────────────────────────┤
//! │ Offset │ Méret  │ Mező           │ Leírás             │
//! ├────────┼────────┼────────────────┼────────────────────┤
//! │   0    │ 8 byte │ transaction_id │ u64 LE             │
//! │   8    │ 1 byte │ entry_type     │ WALEntryType enum  │
//! │   9    │ 4 byte │ data_len       │ u32 LE             │
//! │  13    │ N byte │ data           │ JSON payload       │
//! │ 13+N   │ 4 byte │ checksum       │ CRC32              │
//! └────────┴────────┴────────────────┴────────────────────┘
//!
//! Entry Types (WALEntryType):
//! ┌───────┬─────────────────┬──────────────────────────────┐
//! │ Kód   │ Típus           │ Jelentés                     │
//! ├───────┼─────────────────┼──────────────────────────────┤
//! │ 0x01  │ Begin           │ Tranzakció kezdete           │
//! │ 0x02  │ Operation       │ Insert/Update/Delete művelet │
//! │ 0x03  │ Commit          │ Tranzakció véglegesítése     │
//! │ 0x04  │ Abort           │ Tranzakció visszavonása      │
//! │ 0x05  │ IndexChange     │ Index módosítás              │
//! │ 0x06  │ MetadataSnapshot│ Metadata mentés recovery-hez │
//! └───────┴─────────────────┴──────────────────────────────┘
//! ```
//!
//! ## Invariánsok és Garancia
//!
//! 1. **Write-Ahead Szabály**: WAL MINDIG előbb íródik, mint az adat
//!    - `WAL::append()` → `WAL::flush()` → Storage write
//!
//! 2. **Atomicity**: Egy tranzakció vagy TELJES EGÉSZÉBEN látható, vagy EGYÁLTALÁN NEM
//!    - Recovery csak COMMIT-olt tranzakciókat játszik vissza
//!    - Félbeszakadt tranzakciók automatikusan eldobódnak
//!
//! 3. **Durability**: `fsync()` után az adat GARANTÁLTAN lemezen van
//!    - `WAL::flush()` = `file.sync_all()` = kernel buffer → disk
//!    - Áramszünet után is megmarad
//!
//! 4. **CRC32 Checksum**: Minden entry ellenőrzött
//!    - Sérült entry → `IronBaseError::WALCorruption`
//!    - Félbehagyott írás detektálható
//!
//! 5. **Streaming Recovery**: O(aktív tranzakciók) memória
//!    - `WALEntryIterator`: entry-nként olvas
//!    - `TransactionGrouper`: csak aktív TX-eket tartja memóriában
//!
//! ## Crash Szcenáriók
//!
//! | Crash Pont                    | Eredmény                          |
//! |-------------------------------|-----------------------------------|
//! | WAL append közben             | Entry eldobva (checksum hiba)     |
//! | WAL flush előtt               | Entry elveszett (memóriában volt) |
//! | WAL flush után, storage előtt | Recovery újrajátssza              |
//! | Storage flush után            | WAL törlődik, minden OK           |
//!
//! ## Kapcsolódó Modulok
//!
//! - [`crate::transaction`] - Tranzakció állapotgép (Begin/Commit/Abort)
//! - [`crate::storage`] - Tényleges dokumentum tárolás
//! - [`crate::recovery`] - Magasabb szintű crash recovery
//! - [`crate::database::durability`] - Durability módok (Safe/Batch/Unsafe)
//!
//! ## Példa Használat
//!
//! ```rust,ignore
//! // WAL írás (low-level)
//! let mut wal = WriteAheadLog::open("data.wal")?;
//! wal.append(&WALEntry::new(tx_id, WALEntryType::Begin, vec![]))?;
//! wal.append(&WALEntry::new(tx_id, WALEntryType::Operation, json_bytes))?;
//! wal.append(&WALEntry::new(tx_id, WALEntryType::Commit, vec![]))?;
//! wal.flush()?;  // COMMIT POINT - adat most durable
//!
//! // Recovery (startup)
//! let file = File::open("data.wal")?;
//! let iter = WALEntryIterator::new(BufReader::new(file))?;
//! let grouper = TransactionGrouper::new(iter);
//! for tx_result in grouper {
//!     let tx = tx_result?;
//!     // Csak COMMIT-olt tranzakciók - replay operations
//!     for op in tx.operations() {
//!         storage.apply(op)?;
//!     }
//! }
//! wal.clear()?;  // Recovery kész
//! ```
//!
//! ## Submodulok
//!
//! - [`entry`] - `WALEntry` és `WALEntryType` definíciók, serializáció
//! - [`reader`] - `WALEntryIterator` streaming olvasó
//! - [`writer`] - `WriteAheadLog` fájl kezelő (append, flush, clear)
//! - [`recovery`] - `TransactionGrouper` és `CommittedTransaction`

mod entry;
mod reader;
mod recovery;
mod writer;

// NOTE: MAX_WAL_ENTRY_SIZE and WAL_HEADER_SIZE are internal constants, not re-exported
pub use entry::{WALEntry, WALEntryType};
pub use reader::WALEntryIterator;
pub use recovery::{CommittedTransaction, TransactionGrouper};
pub use writer::WriteAheadLog;
