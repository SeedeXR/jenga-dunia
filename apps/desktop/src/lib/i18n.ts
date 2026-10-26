// UI languages: English (the source), Kiswahili and French. Strings are keyed by their English text so
// a missing translation falls back to English. Kiswahili and French are first drafts — corrections
// welcome (docs/user/desktop-app.md). The choice persists in the browser store.

import dict from "./i18n.json";

export type Lang = "en" | "sw" | "fr";
export const LANGS: { code: Lang; label: string }[] = [
  { code: "en", label: "English" },
  { code: "sw", label: "Kiswahili" },
  { code: "fr", label: "Français" },
];

// The translations live in i18n.json, which Studio (apps/studio/src/i18n.rs) embeds as well.
const DICT: Record<Exclude<Lang, "en">, Record<string, string>> = dict;

/** The language stored for this browser (English when none, or storage is blocked). */
export function storedLang(): Lang {
  try {
    const v = localStorage.getItem("jd.lang");
    return v === "sw" || v === "fr" ? v : "en";
  } catch {
    return "en";
  }
}

/** Translate an English UI string; unknown strings come back as given. */
export function translate(lang: Lang, text: string): string {
  return lang === "en" ? text : (DICT[lang][text] ?? text);
}

/** Every key each translation covers (for the completeness test). */
export const KEYS = Object.keys(DICT.sw);
export const dictionary = (lang: Exclude<Lang, "en">) => DICT[lang];
