// Fails when a built frontend carries the update test build's auto install
// (src/updater/UpdaterProvider.tsx, Vite mode "update-test",
// scripts/build-update-test.sh). Runs after every `pnpm build` and after the
// app builds (scripts/build-app.sh, scripts/build-release.sh), so a test
// bundle that installs updates without a click never ships.
//
//   node scripts/check-release-dist.mjs [dist dir]   (default: dist)
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

// src/lib/updater.ts UPDATE_TEST_MARKER, written by the auto install branch.
const AUTO_INSTALL_MARKER = "woodshed-update-test";

function files(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? files(path) : [path];
  });
}

const dir = process.argv[2] ?? "dist";
let found;
try {
  found = files(dir).filter(
    (path) =>
      /\.(js|mjs|html)$/.test(path) && readFileSync(path, "utf8").includes(AUTO_INSTALL_MARKER),
  );
} catch (error) {
  console.error(`check-release-dist: cannot read ${dir}: ${error.message}`);
  process.exit(2);
}
if (found.length > 0) {
  console.error(
    `check-release-dist: ${dir} is an update test build (installs updates without a click): ${found.join(", ")}`,
  );
  process.exit(1);
}
console.log(`check-release-dist: ${dir} has no update test code`);
