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

// 화면 문구 규칙: 구현 상태·정책·기능 소개 문장은 화면에 두지 않습니다(AGENTS.md).
const POLICY_PHRASES = [
  /지원하지 않/,
  /보내지 않/,
  /서버/,
  /로컬/,
  /자동으로/,
  /수 있습니다/,
  /not supported/i,
  /\bunsupported\b/i,
  /\bserver\b/i,
  /\blocal(ly)?\b/i,
  /\bautomatically\b/i,
  /\bnever\b/i,
  /\byou can\b/i,
];

test("no locale string describes a policy or a feature", () => {
  const values = (value: unknown): string[] =>
    typeof value === "object" && value !== null
      ? Object.values(value).flatMap(values)
      : [String(value)];
  const offending = [...values(ko), ...values(en)].filter((text) =>
    POLICY_PHRASES.some((phrase) => phrase.test(text)),
  );
  expect(offending).toEqual([]);
});
