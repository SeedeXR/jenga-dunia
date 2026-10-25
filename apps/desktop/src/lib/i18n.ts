// UI languages: English (the source), Kiswahili and French. Strings are keyed by their English text so
// a missing translation falls back to English. Kiswahili and French are first drafts — corrections
// welcome (docs/user/desktop-app.md). The choice persists in the browser store.

export type Lang = "en" | "sw" | "fr";
export const LANGS: { code: Lang; label: string }[] = [
  { code: "en", label: "English" },
  { code: "sw", label: "Kiswahili" },
  { code: "fr", label: "Français" },
];

const DICT: Record<Exclude<Lang, "en">, Record<string, string>> = {
  sw: {
    Project: "Mradi",
    "Reference image": "Picha ya marejeo",
    Layers: "Tabaka",
    Settings: "Mipangilio",
    Tasks: "Kazi",
    Diagnostics: "Uchunguzi",
    Brush: "Brashi",
    Grid: "Gridi",
    Sun: "Jua",
    Scatter: "Tawanya",
    Actions: "Vitendo",
    "Simulation layers": "Tabaka za uigaji",
    Materials: "Nyenzo",
    "Terrain · Noise": "Ardhi · Kelele",
    Export: "Hamisha",
    Language: "Lugha",
    New: "Mpya",
    Open: "Fungua",
    Save: "Hifadhi",
    Generate: "Tengeneza",
    Evaluate: "Kokotoa",
  },
  fr: {
    Project: "Projet",
    "Reference image": "Image de référence",
    Layers: "Calques",
    Settings: "Réglages",
    Tasks: "Tâches",
    Diagnostics: "Diagnostics",
    Brush: "Pinceau",
    Grid: "Grille",
    Sun: "Soleil",
    Scatter: "Dispersion",
    Actions: "Actions",
    "Simulation layers": "Calques de simulation",
    Materials: "Matériaux",
    "Terrain · Noise": "Terrain · Bruit",
    Export: "Exporter",
    Language: "Langue",
    New: "Nouveau",
    Open: "Ouvrir",
    Save: "Enregistrer",
    Generate: "Générer",
    Evaluate: "Évaluer",
  },
};

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
