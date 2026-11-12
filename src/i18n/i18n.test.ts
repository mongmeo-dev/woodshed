import en from "../locales/en.json";
import ko from "../locales/ko.json";
import { TOOL_IDS } from "../tools/tools";
import { resolveLanguage } from ".";

function keys(value: unknown, prefix = ""): string[] {
  if (typeof value !== "object" || value === null) return [prefix];
  return Object.entries(value).flatMap(([key, child]) =>
    keys(child, prefix ? `${prefix}.${key}` : key),
  );
}

test("ko and en define the same keys", () => {
  expect(keys(ko).sort()).toEqual(keys(en).sort());
});

test("every tool has a name in both languages", () => {
  for (const tool of TOOL_IDS) {
    expect(ko.tools[tool]).toBeTruthy();
    expect(en.tools[tool]).toBeTruthy();
  }
});

test("saved language wins, then the first supported system language", () => {
  expect(resolveLanguage("en", ["ko-KR"])).toBe("en");
  expect(resolveLanguage(null, ["ko-KR", "en-US"])).toBe("ko");
  expect(resolveLanguage(null, ["fr-FR", "en-GB"])).toBe("en");
  expect(resolveLanguage(null, ["ja-JP"])).toBe("en");
  expect(resolveLanguage(null, [])).toBe("en");
});
