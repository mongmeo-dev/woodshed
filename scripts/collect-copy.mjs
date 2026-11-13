#!/usr/bin/env node
// 화면 문구를 한곳에 모아 점검합니다.
//   node scripts/collect-copy.mjs            → 키·한국어·영어 표(Markdown)를 stdout으로
//   node scripts/collect-copy.mjs --ko-text  → 한국어 문구만 한 줄씩(윤문 입력용)
//   node scripts/collect-copy.mjs --check    → 하드코딩 의심 문구·안 쓰는 키를 찾으면 종료 코드 1
// 로캘(src/locales/*.json), macOS 권한 문구(src-tauri/macos/*.lproj/InfoPlist.strings),
// 화면 코드(src/**/*.tsx)의 JSX 텍스트·접근성 속성 리터럴을 봅니다.
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";

const root = new URL("..", import.meta.url).pathname;
const read = (path) => readFileSync(join(root, path), "utf8");

function flatten(value, prefix = "", out = {}) {
  if (typeof value !== "object" || value === null) {
    out[prefix] = String(value);
    return out;
  }
  for (const [key, child] of Object.entries(value)) {
    flatten(child, prefix ? `${prefix}.${key}` : key, out);
  }
  return out;
}

function plistStrings(path) {
  const out = {};
  for (const match of read(path).matchAll(/"([^"]+)"\s*=\s*"([^"]*)";/g)) {
    out[match[1]] = match[2];
  }
  return out;
}

function walk(dir, files = []) {
  for (const entry of readdirSync(join(root, dir), { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) walk(path, files);
    else if (/\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)) files.push(path);
  }
  return files;
}

const ko = flatten(JSON.parse(read("src/locales/ko.json")));
const en = flatten(JSON.parse(read("src/locales/en.json")));
const plistKo = plistStrings("src-tauri/macos/ko.lproj/InfoPlist.strings");
const plistEn = plistStrings("src-tauri/macos/en.lproj/InfoPlist.strings");

const rows = Object.keys(ko).map((key) => ({ key, ko: ko[key], en: en[key] ?? "" }));
for (const key of Object.keys(plistKo)) {
  rows.push({ key: `InfoPlist.${key}`, ko: plistKo[key], en: plistEn[key] ?? "" });
}

// 언어 이름만 하드코딩을 허용합니다(AGENTS.md 화면 문구 규칙).
const ALLOWED_LITERALS = new Set(["한국어", "English"]);
const sources = walk("src").map((path) => ({ path, text: read(path) }));

function hardcoded() {
  const found = [];
  for (const { path, text } of sources) {
    if (!path.endsWith(".tsx")) continue;
    text.split("\n").forEach((line, index) => {
      const hits = [
        // JSX 텍스트: >문구<
        ...[
          ...line.matchAll(/(?<![=-])>\s*([^<>{}`;=()]*[A-Za-z가-힣]{2,}[^<>{}`;=()]*?)\s*</g),
        ].map((m) => m[1]),
        // 접근성·툴팁 속성 리터럴
        ...[...line.matchAll(/\b(?:aria-label|title|placeholder|alt)="([^"]+)"/g)].map((m) => m[1]),
      ];
      for (const hit of hits) {
        if (!ALLOWED_LITERALS.has(hit.trim())) {
          found.push(`${relative(root, join(root, path))}:${index + 1}: ${hit.trim()}`);
        }
      }
    });
  }
  return found;
}

function unusedKeys() {
  const code = sources.map((s) => s.text).join("\n");
  return Object.keys(ko).filter((key) => {
    if (code.includes(`"${key}"`) || code.includes(`'${key}'`) || code.includes(`\`${key}\``)) {
      return false;
    }
    // 동적 키(`metronome.accent.${a}` 같은 접두사 + 변수)는 접두사가 쓰이면 쓰는 것으로 봅니다.
    const parts = key.split(".");
    for (let i = parts.length - 1; i > 0; i--) {
      const prefix = parts.slice(0, i).join(".");
      if (code.includes(`${prefix}.\${`) || code.includes(`"${prefix}"`)) return false;
    }
    // useTranslation keyPrefix 등으로 마지막 조각만 쓰는 경우
    const tail = parts.slice(-2).join(".");
    return !(code.includes(`"${tail}"`) || code.includes(`'${tail}'`));
  });
}

const args = new Set(process.argv.slice(2));
if (args.has("--ko-text")) {
  for (const row of rows) console.log(`${row.key}\t${row.ko}`);
} else if (args.has("--check")) {
  const literals = hardcoded();
  const unused = unusedKeys();
  for (const line of literals) console.log(`하드코딩 의심: ${line}`);
  for (const key of unused) console.log(`쓰지 않는 키(확인 필요): ${key}`);
  console.log(
    `문구 ${rows.length}개, 하드코딩 의심 ${literals.length}개, 쓰지 않는 키 ${unused.length}개`,
  );
  process.exitCode = literals.length > 0 ? 1 : 0;
} else {
  const cell = (text) => text.replaceAll("|", "\\|");
  console.log("| 키 | 한국어 | English |\n| --- | --- | --- |");
  for (const row of rows) console.log(`| \`${row.key}\` | ${cell(row.ko)} | ${cell(row.en)} |`);
}
