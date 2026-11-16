import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, expect, test } from "vitest";
import { UPDATE_TEST_MARKER } from "../lib/updater";

// The release entry points refuse every way of giving `tauri build` another
// config, before they touch a keychain or start a build. A stand-in `pnpm`
// records whether a build would have started.
const ENTRY_POINTS = ["scripts/build-app.sh", "scripts/build-release.sh"];
let work: string;
let fakeBin: string;
let started: string;
let marked: string;
let markedConfig: string;

beforeAll(() => {
  work = mkdtempSync(join(tmpdir(), "woodshed-entry-"));
  fakeBin = join(work, "bin");
  started = join(work, "build-started");
  mkdirSync(fakeBin);
  writeFileSync(join(fakeBin, "pnpm"), `#!/bin/sh\ntouch "${started}"\nexit 0\n`);
  chmodSync(join(fakeBin, "pnpm"), 0o755);
  // A clean dist/ would pass the dist check; the test frontend sits elsewhere.
  mkdirSync(join(work, "dist", "assets"), { recursive: true });
  writeFileSync(join(work, "dist", "assets", "index.js"), "finally{c.current=null}");
  marked = join(work, "update-test-dist");
  mkdirSync(join(marked, "assets"), { recursive: true });
  writeFileSync(join(marked, "index.html"), "<!doctype html>");
  writeFileSync(
    join(marked, "assets", "index.js"),
    `t&&(document.documentElement.dataset.updateTest=\`${UPDATE_TEST_MARKER}\`,await d())`,
  );
  markedConfig = join(work, "update-test.conf.json");
  writeFileSync(
    markedConfig,
    JSON.stringify({ build: { frontendDist: marked, beforeBuildCommand: "" } }),
  );
});

afterAll(() => {
  rmSync(work, { recursive: true, force: true });
});

function run(script: string, args: string[], env: Record<string, string> = {}) {
  rmSync(started, { force: true });
  const clean = Object.fromEntries(
    Object.entries(process.env).filter(([name]) => !name.startsWith("TAURI_")),
  );
  const result = spawnSync("sh", [script, ...args], {
    encoding: "utf8",
    env: { ...clean, ...env, PATH: `${fakeBin}:${process.env.PATH}` },
  });
  return { status: result.status, stderr: result.stderr, started: existsSync(started) };
}

const CONFIG_ARGS = [
  ["--config", "CONFIG"],
  ["--config=CONFIG"],
  ["-c", "CONFIG"],
  ["-cCONFIG"],
  ["-dc", "CONFIG"],
  ["--verbose", "--config", "CONFIG"],
  ["--", "--config", "CONFIG"],
];

test.each(ENTRY_POINTS)(
  "%s refuses a --config that points the frontend at another dist",
  (script) => {
    for (const args of CONFIG_ARGS) {
      const result = run(
        script,
        args.map((arg) => arg.replace("CONFIG", markedConfig)),
      );
      expect(result.status, args.join(" ")).toBe(2);
      expect(result.stderr).toContain("takes no --config/-c");
      expect(result.started, args.join(" ")).toBe(false);
    }
  },
);

test.each(ENTRY_POINTS)("%s refuses config from the environment", (script) => {
  for (const name of ["TAURI_CONFIG", "TAURI_FRONTEND_PATH", "TAURI_APP_PATH", "TAURI_PATH"]) {
    const result = run(script, [], {
      [name]: name === "TAURI_CONFIG" ? `{"build":{"frontendDist":"${marked}"}}` : marked,
    });
    expect(result.status, name).toBe(2);
    expect(result.stderr).toContain(`unset ${name}`);
    expect(result.started, name).toBe(false);
  }
});

test.each(ENTRY_POINTS)("%s refuses a macOS platform config the CLI would merge", (script) => {
  const platform = "src-tauri/tauri.macos.conf.json";
  expect(existsSync(platform)).toBe(false);
  writeFileSync(platform, JSON.stringify({ build: { frontendDist: marked } }));
  try {
    const result = run(script, []);
    expect(result.status).toBe(2);
    expect(result.stderr).toContain("tauri.macos.conf.json would be merged");
    expect(result.started).toBe(false);
  } finally {
    rmSync(platform, { force: true });
  }
});
