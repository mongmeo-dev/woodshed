import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, expect, test } from "vitest";
import { AUTO_INSTALL, UPDATE_TEST_MODE } from "../lib/updater";

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
  const marker = /console\.info\("(woodshed-update-test)[^"]*"\)/.exec(
    readFileSync("src/updater/UpdaterProvider.tsx", "utf8"),
  )?.[1];
  expect(marker).toBeDefined();
  expect(readFileSync(SCRIPT, "utf8")).toContain(`"${marker}"`);

  expect(checkDist(dist(`t&&(console.info(\`${marker}: installing\`),await d())`))).toBe(1);
  expect(checkDist(dist("finally{c.current=null}"))).toBe(0);
});
