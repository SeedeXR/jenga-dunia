import { describe, expect, it } from "vitest";
import { KEYS, dictionary, translate } from "./i18n";

describe("UI languages", () => {
  it("both translations cover the same keys, with non-empty strings", () => {
    for (const lang of ["sw", "fr"] as const) {
      const d = dictionary(lang);
      expect(Object.keys(d).sort()).toEqual([...KEYS].sort());
      for (const v of Object.values(d)) expect(v.trim().length).toBeGreaterThan(0);
    }
  });
  it("translates known strings and passes unknown ones through", () => {
    expect(translate("sw", "Project")).toBe("Mradi");
    expect(translate("fr", "Sun")).toBe("Soleil");
    expect(translate("en", "Sun")).toBe("Sun");
    expect(translate("sw", "No such string")).toBe("No such string");
  });
});
