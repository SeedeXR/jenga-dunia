//! UI languages, as the desktop's (`apps/desktop/src/lib/i18n.ts`): English is the source;
//! Kiswahili and French come from the desktop's `i18n.json`, embedded here, keyed by the English
//! text so a missing translation falls back to English. The language lives in the egui context
//! (each window and test harness its own); the section titles and the Language label follow it.

use std::collections::HashMap;
use std::sync::OnceLock;

use eframe::egui::{Context, Id};

/// The desktop's `LANGS`: code and label.
pub const LANGS: [(&str, &str); 3] = [("en", "English"), ("sw", "Kiswahili"), ("fr", "Français")];

type Dict = HashMap<String, HashMap<String, String>>;

fn dict() -> &'static Dict {
    static DICT: OnceLock<Dict> = OnceLock::new();
    DICT.get_or_init(|| {
        serde_json::from_str(include_str!("../../desktop/src/lib/i18n.json")).unwrap_or_default()
    })
}

fn key() -> Id {
    Id::unique("jd-lang")
}

/// Use `lang` (`en`, `sw`, `fr`) for this context's UI.
pub fn set(ctx: &Context, lang: &str) {
    ctx.data_mut(|d| d.insert_temp(key(), lang.to_owned()));
}

/// This context's language (`en` until set).
pub fn lang(ctx: &Context) -> String {
    ctx.data(|d| d.get_temp::<String>(key()))
        .unwrap_or_else(|| "en".to_owned())
}

/// `text` in this context's language; unknown strings come back as given.
pub fn tr(ctx: &Context, text: &str) -> String {
    let lang = lang(ctx);
    dict()
        .get(&lang)
        .and_then(|d| d.get(text))
        .cloned()
        .unwrap_or_else(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_like_the_desktop_and_passes_unknown_strings_through() {
        let ctx = Context::default();
        assert_eq!(tr(&ctx, "Sun"), "Sun");
        set(&ctx, "sw");
        assert_eq!(tr(&ctx, "Project"), "Mradi");
        set(&ctx, "fr");
        assert_eq!(tr(&ctx, "Sun"), "Soleil");
        assert_eq!(tr(&ctx, "No such string"), "No such string");
        // Both translations cover the same keys (the desktop's completeness test, on the same file).
        let (sw, fr) = (&dict()["sw"], &dict()["fr"]);
        let mut a: Vec<_> = sw.keys().collect();
        let mut b: Vec<_> = fr.keys().collect();
        a.sort();
        b.sort();
        assert_eq!(a, b);
    }
}
