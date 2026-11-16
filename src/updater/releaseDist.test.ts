import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, expect, test } from "vitest";
import { AUTO_INSTALL, UPDATE_TEST_MARKER, UPDATE_TEST_MODE } from "../lib/updater";

const SCRIPT = "scripts/check-release-dist.mjs";
const dirs: string[] = [];

function dist(code: string) {
  const dir = mkdtempSync(join(tmpdir(), "woodshed-dist-"));
  dirs.push(dir);
  mkdirSync(join(dir, "assets"));
  writeFileSync(join(dir, "index.html"), "<!doctype html>");
  writeFileSync(join(dir, "assets", "index.js"), code);
  return dir;
}

function checkDist(dir: string) {
  return spawnSync(process.execPath, [SCRIPT, dir], { encoding: "utf8" }).status;
}

afterEach(() => {
  for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

test("auto install is off outside the update test mode", () => {
  expect(import.meta.env.MODE).not.toBe(UPDATE_TEST_MODE);
  expect(AUTO_INSTALL).toBe(false);
});

test("the release check refuses a frontend with the auto install code", () => {
  // The marker is a DOM write in the branch, not a log a minifier may drop.
  expect(readFileSync("src/updater/UpdaterProvider.tsx", "utf8")).toMatch(
    /if \(found && AUTO_INSTALL\) \{[^}]*document\.documentElement\.dataset\.updateTest = UPDATE_TEST_MARKER;[^}]*await install\(\);/,
  );
  expect(readFileSync(SCRIPT, "utf8")).toContain(`"${UPDATE_TEST_MARKER}"`);

  // As the minifier leaves it, with console calls dropped.
  expect(
    checkDist(
      dist(
        `var On=\`${UPDATE_TEST_MARKER}\`;t&&(document.documentElement.dataset.updateTest=On,await d())`,
      ),
    ),
  ).toBe(1);
  expect(checkDist(dist("finally{c.current=null}"))).toBe(0);
});

test("the app's frontend is dist/ built and checked by pnpm build, and the entry point configs leave it alone", () => {
  const base = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
  expect(base.build.frontendDist).toBe("../dist");
  expect(base.build.beforeBuildCommand).toMatch(/&& pnpm build$/);
  const scripts = JSON.parse(readFileSync("package.json", "utf8")).scripts;
  expect(scripts.build).toMatch(/&& node scripts\/check-release-dist\.mjs$/);
  for (const file of ["tauri.local-signing.conf.json", "tauri.release.conf.json"]) {
    const config = JSON.parse(readFileSync(`src-tauri/${file}`, "utf8"));
    expect(config.build?.frontendDist, file).toBeUndefined();
    expect(config.build?.beforeBuildCommand, file).toBeUndefined();
    expect(config.build?.devUrl, file).toBeUndefined();
    expect(config.plugins?.updater, file).toBeUndefined();
  }
});
