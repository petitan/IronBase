//! Hungarian texts

use super::Messages;

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
        "Kiválaszt"
    }
    fn cmd_page(&self) -> &'static str {
        "Lapoz"
    }
    fn cmd_scroll(&self) -> &'static str {
        "Görget"
    }
    fn cmd_refresh(&self) -> &'static str {
        "Frissít"
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
        "Szerkeszt"
    }
    fn cmd_delete(&self) -> &'static str {
        "Töröl"
    }
    fn cmd_copy(&self) -> &'static str {
        "Másol"
    }
    fn cmd_help(&self) -> &'static str {
        "Súgó"
    }
    fn cmd_quit(&self) -> &'static str {
        "Kilépés"
    }
    fn bar_error(&self, error: &str) -> String {
        format!(" | Hiba: {error}")
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
            " [SZŰRT – Esc töröl]"
        } else {
            ""
        };
        format!(" Dokumentumok ({page}/{pages}. oldal){filter} ")
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
}
