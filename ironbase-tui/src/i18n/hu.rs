//! Hungarian texts

use super::{Messages, Op, ToolCategory};
use crate::state::filter::{FilterOperator, SortDirection};
use crate::state::query::QueryTemplate;
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

    fn filter_title(&self) -> &'static str {
        "Szűrő"
    }
    fn key_choose_field(&self) -> &'static str {
        "Mezőválasztás"
    }
    fn key_next(&self) -> &'static str {
        "Tovább"
    }
    fn field_title(&self, count: usize) -> String {
        if count == 0 {
            " Mező ".to_string()
        } else {
            format!(" Mező ({count} mező) [↑↓] ")
        }
    }
    fn field_placeholder_examples(&self) -> &'static str {
        "pl. name, age, email..."
    }
    fn field_placeholder_pick(&self) -> &'static str {
        "gépelj, vagy válassz a nyilakkal..."
    }
    fn label_operator(&self) -> &'static str {
        "Operátor"
    }
    fn label_value(&self) -> &'static str {
        "Érték"
    }
    fn value_placeholder(&self) -> &'static str {
        "keresett érték..."
    }
    fn hint_type_field(&self) -> &'static str {
        "Írj be egy mezőnevet (pl. name, age, email)"
    }
    fn hint_field_next(&self) -> &'static str {
        "Enter/Tab: tovább az operátorra"
    }
    fn hint_operator(&self) -> &'static str {
        "←/→: operátorválasztás, Enter/Tab: tovább"
    }
    fn hint_add_exists(&self) -> &'static str {
        "Enter: feltétel hozzáadása (érték nem kell)"
    }
    fn hint_type_value(&self) -> &'static str {
        "Írd be a keresett értéket, vagy ^S: azonnali keresés"
    }
    fn hint_add_condition(&self) -> &'static str {
        "Enter: feltétel hozzáadása | ^S: azonnali keresés"
    }
    fn hint_filters(&self) -> &'static str {
        "Enter: szerkesztés | Del: törlés | Ctrl+↑↓: mozgatás | ^S: keresés"
    }
    fn hint_sort(&self) -> &'static str {
        "←/→: mezőválasztás | Space: irány | ^S: keresés"
    }
    fn active_filters_title(&self, count: usize) -> String {
        format!(" Aktív szűrők ({count}) ")
    }
    fn no_active_filters(&self) -> &'static str {
        "Nincs aktív szűrő – adj hozzá egyet!"
    }
    fn label_sort(&self) -> &'static str {
        "Rendezés"
    }
    fn no_sort(&self) -> &'static str {
        "Nincs rendezés"
    }
    fn press_ctrl_s_to_search(&self) -> &'static str {
        "Nyomd meg a ^S-t a kereséshez"
    }
    fn add_filters(&self) -> &'static str {
        "Adj hozzá szűrőket"
    }
    fn query_title(&self) -> &'static str {
        "Lekérdezés-építő"
    }
    fn key_run(&self) -> &'static str {
        "Futtatás"
    }
    fn key_plan(&self) -> &'static str {
        "Terv"
    }
    fn key_templates(&self) -> &'static str {
        "Sablonok"
    }
    fn query_results_header(&self, count: usize, total: usize) -> String {
        if total > count {
            format!("Találatok ({count}/{total}) [^j/k] [Enter: szűrő]:")
        } else {
            format!("Találatok ({count}) [^j/k] [Enter: szűrő]:")
        }
    }
    fn label_result(&self) -> &'static str {
        "Eredmény"
    }
    fn query_prompt(&self) -> &'static str {
        "Írj egy lekérdezést, és nyomd meg a ^S-t (Ctrl+S)"
    }
    fn query_status(&self, line: usize, column: usize, collection: &str, plan: bool) -> String {
        {
            let plan = if plan { " [terv]" } else { "" };
            format!("Sor: {line} Oszlop: {column} | Kollekció: {collection}{plan}")
        }
    }
    fn query_plan_title(&self) -> &'static str {
        "Lekérdezési terv"
    }
    fn label_plan(&self) -> &'static str {
        "Terv"
    }
    fn label_index(&self) -> &'static str {
        "Index"
    }
    fn label_estimated_rows(&self) -> &'static str {
        "Becsült sorok"
    }
    fn label_stage(&self) -> &'static str {
        "Szakasz"
    }
    fn label_cost(&self) -> &'static str {
        "Költség"
    }
    fn label_reason(&self) -> &'static str {
        "Indoklás"
    }
    fn no_query_plan(&self) -> &'static str {
        "Nincs lekérdezési terv. Nyomd meg a ^E-t."
    }
    fn query_plan_toggle_hint(&self) -> &'static str {
        "[^E] Váltás az eredmények és a terv között"
    }
    fn server_info_title(&self) -> &'static str {
        "Szerverinfó [j/k: görgetés, Esc: bezárás]"
    }
    fn loading(&self) -> &'static str {
        "Betöltés..."
    }
    fn database_info(&self) -> &'static str {
        "Adatbázis-információk"
    }
    fn label_server(&self) -> &'static str {
        "Szerver"
    }
    fn label_api_key(&self) -> &'static str {
        "API-kulcs"
    }
    fn api_key_required(&self) -> &'static str {
        "Kötelező"
    }
    fn api_key_not_required(&self) -> &'static str {
        "Nem szükséges"
    }
    fn label_database(&self) -> &'static str {
        "Adatbázis"
    }
    fn label_file_size(&self) -> &'static str {
        "Fájlméret"
    }
    fn label_collection_count(&self) -> &'static str {
        "Kollekciók száma"
    }
    fn label_total_documents(&self) -> &'static str {
        "Összes dokumentum"
    }
    fn label_total_indexes(&self) -> &'static str {
        "Összes index"
    }
    fn label_of_which_vector(&self) -> &'static str {
        "ebből vektor"
    }
    fn vector_index_summary(&self, indexes: usize, vectors: usize) -> String {
        format!("{indexes} ({vectors} vektor)")
    }
    fn label_collections(&self) -> &'static str {
        "Kollekciók"
    }
    fn available_tools(&self, count: usize) -> String {
        format!("Elérhető MCP-eszközök ({count})")
    }
    fn available_prompts(&self, count: usize) -> String {
        format!("Elérhető MCP-promptok ({count})")
    }
    fn no_description(&self) -> &'static str {
        "Nincs leírás"
    }
    fn protocol_version(&self, version: &str) -> String {
        format!("MCP-protokollverzió: {version}")
    }

    fn filter_operator(&self, operator: FilterOperator) -> &'static str {
        match operator {
            FilterOperator::Equals => "=",
            FilterOperator::NotEquals => "≠",
            FilterOperator::GreaterThan => ">",
            FilterOperator::LessThan => "<",
            FilterOperator::GreaterOrEq => ">=",
            FilterOperator::LessOrEq => "<=",
            FilterOperator::Contains => "tartalmazza",
            FilterOperator::StartsWith => "ezzel kezdődik",
            FilterOperator::Exists => "létezik",
            FilterOperator::In => "listában",
        }
    }
    fn sort_direction(&self, direction: SortDirection) -> &'static str {
        match direction {
            SortDirection::Asc => "↑ növekvő",
            SortDirection::Desc => "↓ csökkenő",
        }
    }
    fn query_template(&self, template: QueryTemplate) -> &'static str {
        match template {
            QueryTemplate::All => "Mind",
            QueryTemplate::ById => "_id alapján",
            QueryTemplate::Equals => "Egyenlőség",
            QueryTemplate::GreaterThan => "Nagyobb mint",
            QueryTemplate::LessThan => "Kisebb mint",
            QueryTemplate::In => "Listában",
            QueryTemplate::Exists => "Létezik",
            QueryTemplate::Regex => "Regex",
            QueryTemplate::And => "ÉS",
            QueryTemplate::Or => "VAGY",
        }
    }
    fn tool_category(&self, category: ToolCategory) -> &'static str {
        match category {
            ToolCategory::Database => "Adatbázis",
            ToolCategory::Collections => "Kollekciók",
            ToolCategory::Query => "Lekérdezés",
            ToolCategory::Insert => "Beszúrás",
            ToolCategory::Update => "Módosítás",
            ToolCategory::Delete => "Törlés",
            ToolCategory::Count => "Számlálás",
            ToolCategory::Aggregate => "Aggregáció",
            ToolCategory::Indexes => "Indexek",
            ToolCategory::Schema => "Séma",
            ToolCategory::Scripts => "Scriptek",
            ToolCategory::Fuzzy => "Fuzzy keresés",
            ToolCategory::Other => "Egyéb",
        }
    }

    fn no(&self) -> &'static str {
        "Nem"
    }
    fn key_new(&self) -> &'static str {
        "Új"
    }
    fn key_tabs(&self) -> &'static str {
        "Fülek"
    }
    fn key_next_field(&self) -> &'static str {
        "Következő mező"
    }
    fn label_status(&self) -> &'static str {
        "Állapot"
    }
    fn label_field_name(&self) -> &'static str {
        "Mezőnév"
    }
    fn status_active(&self) -> &'static str {
        "Aktív"
    }
    fn api_keys_title(&self) -> &'static str {
        "API-kulcsok"
    }
    fn key_revoke(&self) -> &'static str {
        "Visszavonás"
    }
    fn admin_needs_localhost(&self) -> &'static str {
        "(Az adminműveletekhez localhost-kapcsolat kell)"
    }
    fn admin_needs_key(&self) -> &'static str {
        "(Az adminműveletekhez állítsd be az IRONBASE_ADMIN_KEY-t)"
    }
    fn new_key_label(&self) -> &'static str {
        "Új kulcs"
    }
    fn new_key_warning(&self) -> &'static str {
        "(másold most, többé nem jelenik meg!)"
    }
    fn no_api_keys(&self) -> &'static str {
        "Nincs API-kulcs"
    }
    fn label_key(&self) -> &'static str {
        "Kulcs"
    }
    fn status_revoked(&self) -> &'static str {
        "Visszavonva"
    }
    fn enter_new_key_name(&self) -> &'static str {
        "Add meg az új API-kulcs nevét:"
    }
    fn confirm_revoke_key(&self, name: &str) -> String {
        format!("Biztosan visszavonod a(z) „{name}” API-kulcsot?")
    }
    fn confirm_delete_key(&self, name: &str) -> String {
        format!("Biztosan TÖRLÖD a(z) „{name}” API-kulcsot?")
    }
    fn listeners_title(&self) -> &'static str {
        "Listenerek"
    }
    fn key_enable(&self) -> &'static str {
        "Engedélyezés"
    }
    fn key_disable(&self) -> &'static str {
        "Letiltás"
    }
    fn restart_needed(&self) -> &'static str {
        "A változások a szerver újraindítása után lépnek életbe"
    }
    fn no_listeners(&self) -> &'static str {
        "Nincs beállított listener"
    }
    fn label_bind(&self) -> &'static str {
        "Cím"
    }
    fn status_disabled(&self) -> &'static str {
        "Letiltva"
    }
    fn enter_listener_details(&self) -> &'static str {
        "Add meg a listener adatait (Tab: mezőváltás):"
    }
    fn label_bind_address(&self) -> &'static str {
        "Kötési cím"
    }
    fn checkbox_enabled(&self) -> &'static str {
        "[x] Engedélyezve"
    }
    fn checkbox_disabled(&self) -> &'static str {
        "[ ] Letiltva"
    }
    fn space_to_toggle(&self) -> &'static str {
        "(Space: váltás)"
    }
    fn confirm_delete_listener(&self, id: &str) -> String {
        format!("Biztosan TÖRLÖD a(z) „{id}” listenert?")
    }
    fn confirm_enable_listener(&self, id: &str) -> String {
        format!("Biztosan engedélyezed a(z) „{id}” listenert?")
    }
    fn confirm_disable_listener(&self, id: &str) -> String {
        format!("Biztosan letiltod a(z) „{id}” listenert?")
    }
    fn listener_read_only(&self) -> &'static str {
        "Csak olvasható: a listenerek kezeléséhez localhost-kapcsolat kell"
    }
    fn vector_search_title(&self) -> &'static str {
        "Vektorkeresés"
    }
    fn tab_create_index(&self) -> &'static str {
        "Index létrehozása"
    }
    fn tab_list_indexes(&self) -> &'static str {
        "Indexlista"
    }
    fn label_k_limit(&self) -> &'static str {
        "K (korlát)"
    }
    fn label_vector_json(&self) -> &'static str {
        "Vektor (JSON-tömb)"
    }
    fn label_filter_optional(&self) -> &'static str {
        "Szűrő (nem kötelező, JSON)"
    }
    fn vector_results_title(&self, count: usize, loading: bool) -> String {
        if loading {
            " Találatok (betöltés...) ".to_string()
        } else {
            format!(" Találatok ({count}) ")
        }
    }
    fn no_vector_results(&self) -> &'static str {
        "Nincs találat. Add meg a vektort, és nyomd meg az Entert."
    }
    fn key_toggle_metric(&self) -> &'static str {
        "Metrikaváltás"
    }
    fn label_dimension(&self) -> &'static str {
        "Dimenzió (0 = automatikus)"
    }
    fn label_distance_metric(&self) -> &'static str {
        "Távolságmetrika [Space: váltás]"
    }
    fn metric_cosine(&self) -> &'static str {
        "Koszinusz (embeddingekhez ajánlott)"
    }
    fn metric_euclidean(&self) -> &'static str {
        "Euklideszi (L2-távolság)"
    }
    fn metric_dot_product(&self) -> &'static str {
        "Skaláris szorzat (normalizált vektorokhoz)"
    }
    fn dimension_tip(&self) -> &'static str {
        "Tipp: a szokásos embeddingek dimenziója 384, 768 vagy 1536."
    }
    fn vector_indexes_title(&self, count: usize) -> String {
        format!(" Vektorindexek ({count}) ")
    }
    fn no_vector_indexes(&self) -> &'static str {
        "Nincs vektorindex. Az „Index létrehozása” fülön hozhatsz létre egyet."
    }
    fn vector_index_row(
        &self,
        name: &str,
        field: &str,
        dimension: usize,
        metric: &str,
        vectors: usize,
    ) -> String {
        format!("{name} | {field} | dim:{dimension} | {metric} | {vectors} vektor")
    }
    fn index_title(&self) -> &'static str {
        "Indexkezelés"
    }
    fn key_analyze(&self) -> &'static str {
        "Elemzés"
    }
    fn existing_indexes_title(&self, analyzing: bool) -> String {
        if analyzing {
            " Meglévő indexek (elemzés...) ".to_string()
        } else {
            " Meglévő indexek ".to_string()
        }
    }
    fn no_indexes_only_id(&self) -> &'static str {
        "Nincs index (csak _id)"
    }
    fn index_statistics_title(&self) -> &'static str {
        "Indexstatisztika"
    }
    fn no_index_selected(&self) -> &'static str {
        "Nincs kiválasztott index"
    }
    fn load_statistics_hint(&self) -> &'static str {
        "Statisztika betöltése: [a] Elemzés"
    }
    fn staleness_partial(&self) -> &'static str {
        "Részleges"
    }
    fn staleness_stale(&self) -> &'static str {
        "Elavult"
    }
    fn label_keys(&self) -> &'static str {
        "Kulcsok"
    }
    fn label_distinct(&self) -> &'static str {
        "Különböző"
    }
    fn label_histogram(&self) -> &'static str {
        "Hisztogram"
    }
    fn create_index_title(&self) -> &'static str {
        "Új index létrehozása"
    }
    fn label_fields_comma(&self) -> &'static str {
        "Mezők (vesszővel)"
    }
    fn label_compound(&self) -> &'static str {
        "Összetett"
    }
    fn compound_field_count(&self, count: usize) -> String {
        format!(" [{count} mező]")
    }
    fn label_unique(&self) -> &'static str {
        "Egyedi"
    }
    fn label_sparse(&self) -> &'static str {
        "Ritka"
    }
    fn sparse_hint(&self) -> &'static str {
        "(csak a mezőt tartalmazó dokumentumok)"
    }
    fn stale_indexes(&self, count: usize) -> String {
        format!("{count} elavult index – [a] Elemzés")
    }
}
