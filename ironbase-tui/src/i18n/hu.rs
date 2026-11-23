//! Hungarian texts

use super::{Messages, Op};
use std::fmt::Display;

pub struct Hu;

impl Messages for Hu {
    fn header_db(&self, version: &str, db: &str, collections: usize, docs: usize) -> String {
        format!(" IronBase v{version} | {db} ({collections} kollekció, {docs} dokumentum) ")
    }
    fn pane_collections(&self) -> &'static str {
        "Kollekciók"
    }
    fn pane_documents(&self) -> &'static str {
        "Dokumentumok"
    }
    fn pane_detail(&self) -> &'static str {
        "Részletek"
    }

    fn cmd_panel(&self) -> &'static str {
        "Panel"
    }
    fn cmd_navigate(&self) -> &'static str {
        "Mozgás"
    }
    fn cmd_select(&self) -> &'static str {
        "Kiválasztás"
    }
    fn cmd_page(&self) -> &'static str {
        "Lapozás"
    }
    fn cmd_scroll(&self) -> &'static str {
        "Görgetés"
    }
    fn cmd_refresh(&self) -> &'static str {
        "Frissítés"
    }
    fn cmd_filter(&self) -> &'static str {
        "Szűrő"
    }
    fn cmd_search(&self) -> &'static str {
        "Keresés"
    }
    fn cmd_actions(&self) -> &'static str {
        "Műveletek"
    }
    fn cmd_edit(&self) -> &'static str {
        "Szerkesztés"
    }
    fn cmd_delete(&self) -> &'static str {
        "Törlés"
    }
    fn cmd_copy(&self) -> &'static str {
        "Másolás"
    }
    fn cmd_help(&self) -> &'static str {
        "Súgó"
    }
    fn cmd_quit(&self) -> &'static str {
        "Kilépés"
    }
    fn bar_error(&self, error: &str) -> String {
        format!(" Hiba: {error}")
    }
    fn bar_error_details(&self) -> &'static str {
        " Részletek"
    }

    fn collections_title(&self) -> &'static str {
        " Kollekciók "
    }
    fn no_collections(&self) -> &'static str {
        "Nincs kollekció"
    }
    fn indexes_title(&self) -> &'static str {
        " Indexek "
    }
    fn no_indexes(&self) -> &'static str {
        "Nincs index"
    }
    fn documents_title(&self, page: usize, pages: usize, filtered: bool) -> String {
        let filter = if filtered {
            " [SZŰRT – Esc: szűrő törlése]"
        } else {
            ""
        };
        format!(" Dokumentumok (oldal: {page}/{pages}){filter} ")
    }
    fn no_documents(&self) -> &'static str {
        "Nincs dokumentum"
    }
    fn select_collection(&self) -> &'static str {
        "Válassz kollekciót"
    }
    fn detail_title(&self, matches: Option<(usize, usize)>) -> String {
        match matches {
            Some((current, total)) => format!(" Részletek [{current}/{total}] "),
            None => " Részletek ".to_string(),
        }
    }
    fn select_document(&self) -> &'static str {
        "Válassz dokumentumot"
    }

    fn no_database(&self) -> &'static str {
        "Nincs megnyitva adatbázis"
    }
    fn no_collection_selected(&self) -> &'static str {
        "Nincs kiválasztott kollekció"
    }
    fn no_document_selected(&self) -> &'static str {
        "Nincs kiválasztott dokumentum"
    }
    fn no_script_selected(&self) -> &'static str {
        "Nincs kiválasztott script"
    }
    fn no_version_selected(&self) -> &'static str {
        "Nincs kiválasztott verzió"
    }
    fn failed(&self, op: Op, error: &dyn Display) -> String {
        let op = match op {
            Op::Search => "Keresés",
            Op::Refresh => "Frissítés",
            Op::LoadDocuments => "A dokumentumok betöltése",
            Op::LoadSchema => "A séma betöltése",
            Op::LoadScripts => "A scriptlista betöltése",
            Op::LoadScript => "A script betöltése",
            Op::LoadHistory => "Az előzmények betöltése",
            Op::LoadJobs => "A jobok betöltése",
            Op::LoadStatistics => "A statisztika betöltése",
            Op::LoadIndexes => "Az indexek betöltése",
            Op::Save => "Mentés",
            Op::Delete => "Törlés",
            Op::Create => "Létrehozás",
            Op::Rollback => "Visszaállítás",
            Op::Insert => "Beszúrás",
            Op::Update => "Módosítás",
            Op::Query => "Lekérdezés",
            Op::Explain => "A lekérdezési terv",
            Op::Count => "Számlálás",
            Op::Filter => "Szűrés",
            Op::Analyze => "Az indexelemzés",
            Op::Copy => "Másolás a vágólapra",
            Op::File => "Fájlművelet",
            Op::Write => "Írás",
            Op::Flush => "Lemezre írás",
        };
        format!("{op} sikertelen: {error}")
    }
    fn error(&self, error: &dyn Display) -> String {
        format!("Hiba: {error}")
    }
    fn clipboard_unavailable(&self, error: &dyn Display) -> String {
        format!("A vágólap nem érhető el: {error}")
    }
    fn invalid_json(&self, error: &dyn Display) -> String {
        format!("Hibás JSON: {error}")
    }
    fn invalid_json_params(&self, error: &dyn Display) -> String {
        format!("Hibás JSON paraméterek: {error}")
    }
    fn name_required(&self) -> &'static str {
        "Név megadása kötelező"
    }
    fn field_name_required(&self) -> &'static str {
        "Mezőnév megadása kötelező"
    }
    fn file_path_required(&self) -> &'static str {
        "Fájlútvonal megadása kötelező"
    }
    fn invalid_path(&self, path: &str) -> String {
        format!("Érvénytelen útvonal: {path}")
    }
    fn theme_changed(&self, theme: &str) -> String {
        format!("Téma: {theme}")
    }
    fn copied(&self, preview: &str) -> String {
        format!("Vágólapra másolva: {preview}...")
    }

    fn no_matches(&self) -> &'static str {
        "Nincs találat"
    }
    fn match_count(&self, count: usize) -> String {
        format!("{count} találat")
    }
    fn match_count_with_hint(&self, count: usize) -> String {
        format!("{count} találat (n/N: következő/előző)")
    }
    fn match_position(&self, current: usize, total: usize) -> String {
        format!("Találat {current}/{total}")
    }
    fn filter_needs_a_condition(&self) -> &'static str {
        "Adj hozzá legalább egy feltételt"
    }
    fn filter_no_matches(&self, query: &str) -> String {
        format!("0 találat. Lekérdezés: {query}")
    }

    fn collection_exists(&self) -> &'static str {
        "Ez a kollekció már létezik"
    }
    fn collection_created(&self, name: &str) -> String {
        format!("Kollekció létrehozva: {name}")
    }
    fn collection_dropped(&self, name: &str) -> String {
        format!("Kollekció törölve: {name}")
    }
    fn document_deleted(&self) -> &'static str {
        "Dokumentum törölve"
    }
    fn document_not_found(&self) -> &'static str {
        "Dokumentum nem található"
    }
    fn document_unchanged(&self) -> &'static str {
        "Dokumentum változatlan (nincs módosítás)"
    }
    fn document_updated(&self) -> &'static str {
        "Dokumentum frissítve"
    }
    fn document_updated_vector_stale(&self, error: &dyn Display) -> String {
        format!("Dokumentum frissítve, de az újrabeágyazás hibázott (a vektor elavult): {error}")
    }
    fn document_inserted(&self, id: &dyn Display) -> String {
        format!("Dokumentum beszúrva, _id: {id}")
    }
    fn document_must_be_object(&self) -> &'static str {
        "A dokumentumnak objektumnak kell lennie"
    }
    fn document_id_missing(&self) -> &'static str {
        "Hiányzik az eredeti dokumentum _id-ja"
    }
    fn query_found(&self, count: usize, limit: Option<usize>) -> String {
        match limit {
            Some(limit) => format!("{count}+ dokumentum (korlát: {limit})"),
            None => format!("{count} dokumentum található"),
        }
    }
    fn query_plan_loaded(&self) -> &'static str {
        "Lekérdezési terv betöltve"
    }
    fn exported(&self, count: usize, filtered: bool, path: &str) -> String {
        let filtered = if filtered { " (szűrt)" } else { "" };
        format!("{count} dokumentum exportálva{filtered}: {path}")
    }
    fn nothing_to_export(&self) -> &'static str {
        "Nincs exportálandó dokumentum"
    }

    fn primary_index_not_droppable(&self, index: &str) -> String {
        format!("Az elsődleges _id index nem törölhető: {index}")
    }
    fn index_created(&self, fields: &str, compound: bool, flags: &str, name: &str) -> String {
        let kind = if compound {
            "Összetett index"
        } else {
            "Index"
        };
        format!("{kind}: {fields}{flags} létrehozva ({name})")
    }
    fn index_dropped(&self, name: &str) -> String {
        format!("Index törölve: {name}")
    }
    fn analyzing_indexes(&self) -> &'static str {
        "Indexek elemzése..."
    }
    fn index_statistics_refreshed(&self) -> &'static str {
        "Indexstatisztika frissítve"
    }
    fn vector_input_empty(&self) -> &'static str {
        "Nincs megadva vektor"
    }
    fn vector_empty(&self) -> &'static str {
        "A vektor üres"
    }
    fn invalid_vector(&self, error: &dyn Display) -> String {
        format!("Hibás vektor-JSON: {error}")
    }
    fn invalid_filter(&self, error: &dyn Display) -> String {
        format!("Hibás szűrő-JSON: {error}")
    }

    fn auto_embed_disabled(&self) -> &'static str {
        "Automatikus beágyazás kikapcsolva"
    }
    fn search_summary(
        &self,
        found: usize,
        over_budget: usize,
        without_body: usize,
        trimmed: bool,
    ) -> String {
        let found = format!("{found} találat");
        let mut left_out = Vec::new();
        if over_budget > 0 {
            left_out.push(format!("további {over_budget} a kontextuskereten felül"));
        }
        if without_body > 0 {
            left_out.push(format!("{without_body} szövegtörzs nélkül"));
        }
        if left_out.is_empty() && trimmed {
            left_out.push("rövidített szakaszok".to_string());
        }
        if left_out.is_empty() {
            found
        } else {
            format!("{found} (kimaradt: {})", left_out.join(", "))
        }
    }

    fn script_name_required(&self) -> &'static str {
        "Scriptnév megadása kötelező"
    }
    fn script_saved(&self, version: u32) -> String {
        format!("Script mentve (v{version})")
    }
    fn script_deleted(&self, name: &str) -> String {
        format!("Script törölve: {name}")
    }
    fn script_rolled_back(&self, version: u32) -> String {
        format!("Visszaállítva, új verzió: v{version}")
    }
    fn unexpected_search_response(&self) -> &'static str {
        "A keresés nem a kért formában (dokumentumlistaként) válaszolt"
    }
}
