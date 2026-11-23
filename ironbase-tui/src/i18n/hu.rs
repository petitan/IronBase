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

    fn key_save(&self) -> &'static str {
        "Mentés"
    }
    fn key_cancel(&self) -> &'static str {
        "Mégse"
    }
    fn key_close(&self) -> &'static str {
        "Bezárás"
    }
    fn key_create(&self) -> &'static str {
        "Létrehozás"
    }
    fn key_search(&self) -> &'static str {
        "Keresés"
    }
    fn key_search_or_jump(&self) -> &'static str {
        "Keresés/ugrás"
    }
    fn key_format(&self) -> &'static str {
        "Formátum"
    }
    fn key_export(&self) -> &'static str {
        "Exportálás"
    }
    fn key_switch_mode(&self) -> &'static str {
        "Módváltás"
    }
    fn key_switch_field(&self) -> &'static str {
        "Mezőváltás"
    }
    fn key_navigate(&self) -> &'static str {
        "Navigáció"
    }
    fn key_indent(&self) -> &'static str {
        "2 szóköz"
    }
    fn label_error(&self) -> &'static str {
        "Hiba"
    }
    fn label_name(&self) -> &'static str {
        "Név"
    }
    fn label_file(&self) -> &'static str {
        "Fájl"
    }
    fn label_format(&self) -> &'static str {
        "Formátum"
    }
    fn label_path(&self) -> &'static str {
        "Útvonal"
    }
    fn label_mode(&self) -> &'static str {
        "Mód"
    }
    fn label_field(&self) -> &'static str {
        "Mező"
    }
    fn collection_label(&self, name: &str) -> String {
        format!("Kollekció: {name}")
    }
    fn error_details_title(&self) -> &'static str {
        "Hiba részletei"
    }
    fn error_scroll_hint(&self, current: usize, total: usize) -> String {
        format!("({current}/{total}) – ↑/↓: görgetés, Esc: bezárás")
    }
    fn esc_to_close(&self) -> &'static str {
        "Esc: bezárás"
    }
    fn new_collection_title(&self) -> &'static str {
        "Új kollekció"
    }
    fn collection_name_hint(&self) -> &'static str {
        "Adj meg egy nevet (a-z, 0-9, _, -)"
    }
    fn edit_document_title(&self) -> &'static str {
        "Dokumentum szerkesztése"
    }
    fn new_document_title(&self) -> &'static str {
        "Új dokumentum"
    }
    fn cursor_position(&self, line: usize, column: usize) -> String {
        format!("Sor: {line} Oszlop: {column}")
    }
    fn update_check_title(&self) -> &'static str {
        "Frissítés ellenőrzése [Esc: bezárás]"
    }
    fn checking_for_updates(&self) -> &'static str {
        "Frissítés ellenőrzése..."
    }
    fn current_version(&self) -> &'static str {
        "Jelenlegi verzió"
    }
    fn latest_version(&self) -> &'static str {
        "Legújabb verzió"
    }
    fn update_available(&self) -> &'static str {
        "Új verzió érhető el!"
    }
    fn update_instructions(&self) -> &'static str {
        "A frissítéshez futtasd PowerShellben:"
    }
    fn whats_new(&self) -> &'static str {
        "Újdonságok:"
    }
    fn up_to_date(&self) -> &'static str {
        "A legújabb verzió fut!"
    }
    fn export_title(&self) -> &'static str {
        "Exportálás"
    }
    fn export_collection_info(&self, collection: &str, count: usize, filtered: bool) -> String {
        {
            let filtered = if filtered { " – szűrt" } else { "" };
            format!("Kollekció: {collection} ({count} dokumentum{filtered})")
        }
    }
    fn press_enter_to_export(&self) -> &'static str {
        "Nyomd meg az Entert az exportáláshoz"
    }
    fn search_collections_title(&self) -> &'static str {
        "Kollekciók keresése"
    }
    fn search_document_title(&self) -> &'static str {
        "Keresés a dokumentumban"
    }
    fn type_collection_name(&self) -> &'static str {
        "Írj be egy kollekciónevet..."
    }
    fn type_search_text(&self) -> &'static str {
        "Írd be a keresett szöveget..."
    }
    fn collection_with_count(&self, name: &str, documents: usize) -> String {
        format!("{name} ({documents} dokumentum)")
    }
    fn start_typing_to_search(&self) -> &'static str {
        "Kezdj el gépelni a kereséshez"
    }
    fn document_match_help(&self, current: usize, total: usize) -> String {
        format!("Találat: {current}/{total}\n\n[n] Következő  [N] Előző\n[Esc] Bezárás (a találatok kijelölve maradnak)")
    }
    fn open_database_title(&self) -> &'static str {
        "Adatbázis megnyitása"
    }
    fn create_database_title(&self) -> &'static str {
        "Új adatbázis létrehozása"
    }
    fn mode_open(&self) -> &'static str {
        "Megnyitás"
    }
    fn mode_create(&self) -> &'static str {
        "Létrehozás"
    }
    fn open_database_info(&self) -> &'static str {
        "Add meg egy létező adatbázisfájl útvonalát."
    }
    fn create_database_info(&self) -> &'static str {
        "Add meg az új adatbázisfájl útvonalát.\nA fájl még nem létezhet!"
    }
    fn connecting(&self) -> &'static str {
        "Csatlakozás..."
    }
    fn open_database_tip(&self) -> &'static str {
        "Tipp: használj abszolút útvonalat (pl. /home/user/data.mlite)"
    }
    fn create_database_tip(&self) -> &'static str {
        "Tipp: a .mlite kiterjesztés ajánlott"
    }
    fn fulltext_title(&self) -> &'static str {
        "Teljes szöveges keresés"
    }
    fn no_fulltext_index(&self) -> &'static str {
        "Nincs fulltext index – előbb hozz létre egyet."
    }
    fn results_title(&self, count: usize, more: bool) -> String {
        {
            let more = if more { "+" } else { "" };
            format!(" Találatok ({count}{more}) ")
        }
    }
    fn searching(&self) -> &'static str {
        "Keresés..."
    }
    fn confirm_title(&self) -> &'static str {
        "Megerősítés"
    }
    fn confirm_message(&self) -> &'static str {
        "Biztosan folytatod?"
    }
    fn yes(&self) -> &'static str {
        "Igen"
    }
    fn confirm_delete_document_title(&self) -> &'static str {
        "Dokumentum törlése"
    }
    fn confirm_delete_document(&self, collection: &str, preview: &str) -> String {
        format!("Biztosan törlöd a dokumentumot a(z) „{collection}” kollekcióból?\n\n{preview}")
    }
    fn confirm_drop_collection_title(&self) -> &'static str {
        "Kollekció törlése"
    }
    fn confirm_drop_collection(&self, name: &str, documents: usize) -> String {
        format!("Biztosan törlöd a(z) „{name}” kollekciót?\n\n{documents} dokumentum törlődik!")
    }
    fn confirm_drop_index_title(&self) -> &'static str {
        "Index törlése"
    }
    fn confirm_drop_index(&self, index: &str, collection: &str) -> String {
        format!("Biztosan törlöd a(z) „{index}” indexet?\n\nKollekció: {collection}")
    }
    fn confirm_drop_vector_index_title(&self) -> &'static str {
        "Vektorindex törlése"
    }
    fn confirm_drop_vector_index(&self, index: &str, collection: &str) -> String {
        format!("Biztosan törlöd a(z) „{index}” vektorindexet?\n\nKollekció: {collection}")
    }
    fn confirm_rollback_title(&self) -> &'static str {
        "Script visszaállítása"
    }
    fn confirm_rollback(&self, name: &str, version: u32) -> String {
        format!("Biztosan visszaállítod a(z) „{name}” scriptet a v{version} verzióra?")
    }
    fn rollback_button(&self) -> &'static str {
        "Visszaállítás"
    }
    fn confirm_disable_auto_embed_title(&self) -> &'static str {
        "Automatikus beágyazás kikapcsolása"
    }
    fn confirm_disable_auto_embed(&self, collection: &str) -> String {
        format!("Biztosan kikapcsolod az automatikus beágyazást a(z) „{collection}” kollekción?")
    }
    fn disable_button(&self) -> &'static str {
        "Kikapcsolás"
    }
    fn confirm_clear_cache_title(&self) -> &'static str {
        "Beágyazási gyorsítótár ürítése"
    }
    fn confirm_clear_cache(&self) -> &'static str {
        "Biztosan üríted a teljes beágyazási gyorsítótárat?"
    }
    fn clear_button(&self) -> &'static str {
        "Ürítés"
    }
    fn confirm_cancel_job_title(&self) -> &'static str {
        "Beágyazási job megszakítása"
    }
    fn confirm_cancel_job(&self, job_id: &str) -> String {
        format!("Biztosan megszakítod a(z) „{job_id}” beágyazási jobot?")
    }
    fn cancel_job_button(&self) -> &'static str {
        "Megszakítás"
    }
}
