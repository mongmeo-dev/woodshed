// @vitest-environment node
// Runs the push and upload steps of scripts/release-lib.sh against stand-in
// `git` and `gh` (first on PATH) whose answers and failures each test sets,
// so every failure path of a release is fixed without touching GitHub.
import { spawnSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const ORIG = "1111111111111111111111111111111111111111";
const RELEASE = "2222222222222222222222222222222222222222";
const OTHER = "3333333333333333333333333333333333333333";
const TAG_OBJECT = "4444444444444444444444444444444444444444";
const ASSETS = ["Woodshed_0.2.0_aarch64.dmg", "latest.json", "SHA256SUMS"];

// Stand-in git: the state file says what origin has and how a push goes.
const FAKE_GIT = `#!/usr/bin/env node
const fs = require("node:fs");
const file = process.env.MOCK_STATE;
const s = JSON.parse(fs.readFileSync(file, "utf8"));
const args = process.argv.slice(2);
s.calls.push(["git", ...args].join(" "));
const save = () => fs.writeFileSync(file, JSON.stringify(s));
const done = (code, out = "", err = "") => {
  save();
  if (out) process.stdout.write(out);
  if (err) process.stderr.write(err);
  process.exit(code);
};
const [cmd, ...rest] = args;
if (cmd === "push") {
  if (s.push.applies) {
    s.remote.main = rest[2].split(":")[0];
    s.remote.tag = s.local.tag;
  }
  if (s.push.signal) {
    // The release is stopped while git pushes: the shell gets the signal,
    // and with dies, git is killed by it too (Ctrl-C reaches both).
    save();
    process.kill(process.ppid, s.push.signal);
    if (s.push.dies) process.kill(process.pid, s.push.signal);
  }
  done(s.push.code, "", s.push.code ? "fatal: the remote end hung up unexpectedly\\n" : "");
}
if (cmd === "ls-remote") {
  if (s.lsRemoteFails) done(128, "", "fatal: unable to access origin\\n");
  if (s.lsRemoteHangs) {
    save();
    setTimeout(() => process.exit(0), 10000);
    return;
  }
  const lines = [];
  for (const ref of rest.slice(1)) {
    if (ref === "refs/heads/main" && s.remote.main) lines.push(s.remote.main + "\\t" + ref);
    if (ref === "refs/tags/v0.2.0" && s.remote.tag) lines.push("${TAG_OBJECT}\\t" + ref);
    if (ref === "refs/tags/v0.2.0^{}" && s.remote.tag) lines.push(s.remote.tag + "\\t" + ref);
  }
  done(0, lines.map((l) => l + "\\n").join(""));
}
if (cmd === "rev-parse") {
  if (rest[0] === "-q") done(s.local.tag ? 0 : 1, s.local.tag ? s.local.tag + "\\n" : "");
  if (rest[0] === "HEAD") done(0, s.local.head + "\\n");
  if (rest[0] === "--abbrev-ref") done(0, "main\\n");
  if (rest[0] === "--short") done(0, s.local.head.slice(0, 7) + "\\n");
}
if (cmd === "tag" && rest[0] === "-d") {
  s.local.tag = null;
  done(0);
}
if (cmd === "reset") {
  s.local.head = rest[rest.length - 1];
  done(0);
}
if (cmd === "status") done(0);
done(99, "", "fake git: unexpected " + args.join(" ") + "\\n");
`;

// Stand-in gh: one release (or none) and the list of published ones.
const FAKE_GH = `#!/usr/bin/env node
const fs = require("node:fs");
const crypto = require("node:crypto");
const path = require("node:path");
const file = process.env.MOCK_STATE;
const s = JSON.parse(fs.readFileSync(file, "utf8"));
const args = process.argv.slice(2);
s.calls.push(["gh", ...args].join(" "));
const save = () => fs.writeFileSync(file, JSON.stringify(s));
const done = (code, out = "", err = "") => {
  save();
  if (out) process.stdout.write(out);
  if (err) process.stderr.write(err);
  process.exit(code);
};
const fails = (step) => s.fail.includes(step);
const [group, cmd, ...rest] = args;
if (group !== "release") done(99, "", "fake gh: unexpected " + args.join(" ") + "\\n");
if (cmd === "view") {
  if (fails("view")) done(1, "", "HTTP 502: Bad Gateway\\n");
  if (!s.release) done(1, "", "release not found\\n");
  done(0, JSON.stringify(s.release));
}
if (cmd === "list") {
  if (fails("list")) done(1, "", "HTTP 502: Bad Gateway\\n");
  done(0, JSON.stringify(s.published));
}
if (cmd === "create") {
  if (fails("create")) done(1, "", "HTTP 502: Bad Gateway\\n");
  s.release = { isDraft: true, assets: [] };
  done(0);
}
if (cmd === "upload") {
  if (fails("upload")) done(1, "", "upload failed\\n");
  for (const arg of rest.filter((a) => a.startsWith("/"))) {
    const name = path.basename(arg);
    let hex = crypto.createHash("sha256").update(fs.readFileSync(arg)).digest("hex");
    if (s.corruptUpload) hex = "0".repeat(64);
    s.release.assets = s.release.assets.filter((a) => a.name !== name);
    s.release.assets.push({ name, digest: "sha256:" + hex });
  }
  done(0);
}
if (cmd === "edit") {
  if (fails("edit")) done(1, "", "HTTP 502: Bad Gateway\\n");
  s.release.isDraft = false;
  s.latest = rest[0];
  if (fails("edit-after-applying")) done(1, "", "connection reset\\n");
  done(0);
}
done(99, "", "fake gh: unexpected " + args.join(" ") + "\\n");
`;

let dir;

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "release-lib-"));
  mkdirSync(join(dir, "bin"));
  for (const [name, text] of [
    ["git", FAKE_GIT],
    ["gh", FAKE_GH],
  ]) {
    writeFileSync(join(dir, "bin", name), text);
    chmodSync(join(dir, "bin", name), 0o755);
  }
  mkdirSync(join(dir, "out"));
  for (const name of ASSETS) writeFileSync(join(dir, "out", name), `${name} contents\n`);
  writeFileSync(join(dir, "out", "release-notes.md"), "Woodshed 0.2.0\n");
});

afterEach(() => {
  rmSync(dir, { recursive: true, force: true });
});

function writeState(overrides = {}) {
  const state = {
    calls: [],
    local: { head: RELEASE, tag: RELEASE },
    remote: { main: ORIG, tag: null },
    push: { code: 0, applies: true },
    lsRemoteFails: false,
    release: null,
    published: [],
    fail: [],
    corruptUpload: false,
    latest: null,
    ...overrides,
  };
  writeFileSync(join(dir, "state.json"), JSON.stringify(state));
}

const readState = () => JSON.parse(readFileSync(join(dir, "state.json"), "utf8"));

function provenance({ commit = RELEASE, dryRun = false } = {}) {
  const result = spawnSync(
    "node",
    [
      "scripts/release-files.mjs",
      "provenance-write",
      join(dir, "out"),
      "0.2.0",
      commit,
      String(dryRun),
      ...ASSETS,
    ],
    { cwd: ROOT, encoding: "utf8" },
  );
  expect(result.status, result.stderr).toBe(0);
}

/** Runs `body` after sourcing release-lib.sh with a release's variables and its exit trap. */
function run(body, { pushed = "false", checkSeconds = 15 } = {}) {
  const work = join(dir, `work-${Math.random().toString(16).slice(2)}`);
  mkdirSync(work);
  const script = [
    "set -eu",
    ". scripts/release-lib.sh",
    'version=0.2.0 tag=v0.2.0 repo=owner/woodshed out="$OUT" work="$WORK"',
    `assets="${ASSETS.join(" ")}"`,
    `orig_head=${ORIG} release_commit=${RELEASE} bumped=true pushed=${pushed} mounted=false`,
    `pushing=false push_check_seconds=${checkSeconds}`,
    "trap on_exit EXIT",
    "trap 'exit 130' INT TERM HUP",
    body,
  ].join("\n");
  const result = spawnSync("sh", ["-c", script], {
    cwd: ROOT,
    encoding: "utf8",
    env: {
      ...process.env,
      PATH: `${join(dir, "bin")}:${process.env.PATH}`,
      MOCK_STATE: join(dir, "state.json"),
      OUT: join(dir, "out"),
      WORK: work,
    },
  });
  return { ...result, state: readState() };
}

const calls = (state, prefix) => state.calls.filter((call) => call.startsWith(prefix));

describe("push_release", () => {
  it("goes on when origin takes the push", () => {
    writeState();
    const result = run('push_release\necho "pushed=$pushed"');
    expect(result.status, result.stderr).toBe(0);
    expect(result.stdout).toContain("pushed=true");
    expect(calls(result.state, "git tag -d")).toEqual([]);
    expect(calls(result.state, "git reset")).toEqual([]);
  });

  it("keeps the commit and tag when git fails but origin has the tag", () => {
    writeState({ push: { code: 1, applies: true } });
    const result = run("push_release");
    expect(result.status).toBe(1);
    expect(result.stderr).toContain(`origin has tag v0.2.0 at ${RELEASE}`);
    expect(result.stderr).toContain("mise run release -- 0.2.0 --upload-only");
    expect(result.stderr).not.toContain("removing the version commit");
    expect(calls(result.state, "git tag -d")).toEqual([]);
    expect(calls(result.state, "git reset")).toEqual([]);
    expect(result.state.local).toEqual({ head: RELEASE, tag: RELEASE });
  });

  it("undoes the commit and tag when origin took neither ref", () => {
    writeState({ push: { code: 1, applies: false } });
    const result = run("push_release");
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("origin took neither main nor tag v0.2.0");
    expect(calls(result.state, "git tag -d")).toEqual(["git tag -d v0.2.0"]);
    expect(calls(result.state, "git reset")).toEqual([`git reset -q --hard ${ORIG}`]);
    expect(result.state.local).toEqual({ head: ORIG, tag: null });
  });

  it("undoes them when the tag on origin is someone else's and main is not ours", () => {
    writeState({ push: { code: 1, applies: false }, remote: { main: OTHER, tag: OTHER } });
    const result = run("push_release");
    expect(result.status).toBe(1);
    expect(result.state.local).toEqual({ head: ORIG, tag: null });
  });

  it("removes nothing when origin cannot be read after the failure", () => {
    writeState({ push: { code: 1, applies: true }, lsRemoteFails: true });
    const result = run("push_release");
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("cannot tell whether origin took the push");
    expect(result.stderr).toContain("git ls-remote origin refs/heads/main refs/tags/v0.2.0");
    expect(result.stderr).toContain(`git tag -d v0.2.0 && git reset --hard ${ORIG}`);
    expect(calls(result.state, "git tag -d")).toEqual([]);
    expect(calls(result.state, "git reset")).toEqual([]);
  });

  it("removes nothing when origin has main at the version commit but not the tag", () => {
    writeState({ push: { code: 1, applies: false }, remote: { main: RELEASE, tag: null } });
    const result = run("push_release");
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("cannot tell whether origin took the push");
    expect(result.state.local).toEqual({ head: RELEASE, tag: RELEASE });
  });
});

describe("a release stopped during the push", () => {
  const stopped = (push, more = {}) => writeState({ push: { code: 0, ...push }, ...more });

  it("keeps the commit and tag when origin took the push", () => {
    stopped({ applies: true, signal: "SIGTERM" });
    const result = run("push_release\necho not reached");
    expect(result.status).toBe(130);
    expect(result.stdout).not.toContain("not reached");
    expect(result.stderr).toContain("release stopped during the push");
    expect(result.stderr).toContain(`origin has tag v0.2.0 at ${RELEASE}`);
    expect(result.stderr).toContain("mise run release -- 0.2.0 --upload-only");
    expect(calls(result.state, "git tag -d")).toEqual([]);
    expect(calls(result.state, "git reset")).toEqual([]);
    expect(result.state.local).toEqual({ head: RELEASE, tag: RELEASE });
  });

  it("undoes the commit and tag only when origin took neither ref", () => {
    stopped({ applies: false, signal: "SIGTERM" });
    const result = run("push_release");
    expect(result.status).toBe(130);
    expect(result.stderr).toContain("origin took neither main nor tag v0.2.0");
    expect(calls(result.state, "git tag -d")).toEqual(["git tag -d v0.2.0"]);
    expect(calls(result.state, "git reset")).toEqual([`git reset -q --hard ${ORIG}`]);
    expect(result.state.local).toEqual({ head: ORIG, tag: null });
  });

  it("removes nothing when origin cannot be read", () => {
    stopped({ applies: true, signal: "SIGTERM" }, { lsRemoteFails: true });
    const result = run("push_release");
    expect(result.status).toBe(130);
    expect(result.stderr).toContain("cannot tell whether origin took the push");
    expect(calls(result.state, "git tag -d")).toEqual([]);
    expect(calls(result.state, "git reset")).toEqual([]);
    expect(result.state.local).toEqual({ head: RELEASE, tag: RELEASE });
  });

  it("judges origin the same way when Ctrl-C stops git as well", () => {
    stopped({ applies: true, signal: "SIGINT", dies: true });
    const result = run("push_release");
    expect(result.status).toBe(130);
    expect(result.stderr).toContain(`origin has tag v0.2.0 at ${RELEASE}`);
    expect(result.state.local).toEqual({ head: RELEASE, tag: RELEASE });
  });

  it("gives up on an origin that does not answer in time and removes nothing", () => {
    stopped({ applies: false, signal: "SIGTERM" }, { lsRemoteHangs: true });
    const started = Date.now();
    const result = run("push_release", { checkSeconds: 1 });
    expect(Date.now() - started).toBeLessThan(8000);
    expect(result.status).toBe(130);
    expect(result.stderr).toContain("origin could not be read within 1s");
    expect(calls(result.state, "git tag -d")).toEqual([]);
    expect(calls(result.state, "git reset")).toEqual([]);
  });
});

describe("upload_or_explain", () => {
  const pushedTag = { remote: { main: RELEASE, tag: RELEASE } };

  it("publishes only after every file is on the draft", () => {
    writeState(pushedTag);
    provenance();
    const result = run("upload_or_explain", { pushed: "true" });
    expect(result.status, result.stderr).toBe(0);
    const gh = calls(result.state, "gh ").map((call) => call.split(" ").slice(0, 3).join(" "));
    expect(gh).toEqual([
      "gh release view",
      "gh release list",
      "gh release create",
      "gh release upload",
      "gh release view",
      "gh release edit",
    ]);
    expect(calls(result.state, "gh release create")[0]).toContain("--draft");
    expect(calls(result.state, "gh release edit")[0]).toContain("--draft=false --latest");
    expect(result.state.release.isDraft).toBe(false);
    expect(result.state.release.assets.map((asset) => asset.name).sort()).toEqual(
      [...ASSETS].sort(),
    );
  });

  it("continues a draft an earlier attempt left", () => {
    writeState({ ...pushedTag, release: { isDraft: true, assets: [] } });
    provenance();
    const result = run("upload_or_explain", { pushed: "true" });
    expect(result.status, result.stderr).toBe(0);
    expect(calls(result.state, "gh release create")).toEqual([]);
    expect(result.state.release.isDraft).toBe(false);
  });

  it("leaves the draft unpublished and says how to retry when the upload fails", () => {
    writeState({ ...pushedTag, fail: ["upload"] });
    provenance();
    const result = run("upload_or_explain", { pushed: "true" });
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("mise run release -- 0.2.0 --upload-only");
    expect(calls(result.state, "gh release edit")).toEqual([]);
    expect(result.state.release.isDraft).toBe(true);
  });

  it("does not publish files that arrived different", () => {
    writeState({ ...pushedTag, corruptUpload: true });
    provenance();
    const result = run("upload_or_explain", { pushed: "true" });
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("differs");
    expect(calls(result.state, "gh release edit")).toEqual([]);
  });

  it("counts a release published by a lost answer as done on the retry", () => {
    writeState({ ...pushedTag, fail: ["edit-after-applying"] });
    provenance();
    const first = run("upload_or_explain", { pushed: "true" });
    expect(first.status).toBe(1);
    expect(first.stderr).toContain("--upload-only");
    const state = readState();
    writeState({ ...pushedTag, release: state.release });
    const retry = run("upload_or_explain", { pushed: "true" });
    expect(retry.status, retry.stderr).toBe(0);
    expect(retry.stdout).toContain("already published with these files");
    expect(calls(retry.state, "gh release upload")).toEqual([]);
    expect(calls(retry.state, "gh release edit")).toEqual([]);
  });

  it("never changes a release published with other files", () => {
    writeState({
      ...pushedTag,
      release: {
        isDraft: false,
        assets: ASSETS.map((name) => ({ name, digest: `sha256:${"0".repeat(64)}` })),
      },
    });
    provenance();
    const result = run("upload_or_explain", { pushed: "true" });
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("already published with other files");
    expect(calls(result.state, "gh release upload")).toEqual([]);
    expect(calls(result.state, "gh release edit")).toEqual([]);
  });

  it("does not make an older version latest", () => {
    writeState({
      ...pushedTag,
      published: [{ tagName: "v0.3.0", isDraft: false, isPrerelease: false }],
    });
    provenance();
    const result = run("upload_or_explain", { pushed: "true" });
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("v0.3.0 is already published");
    expect(calls(result.state, "gh release create")).toEqual([]);
    expect(calls(result.state, "gh release upload")).toEqual([]);
    expect(calls(result.state, "gh release edit")).toEqual([]);
  });

  it("creates nothing when GitHub cannot say whether the release exists", () => {
    writeState({ ...pushedTag, fail: ["view"] });
    provenance();
    const result = run("upload_or_explain", { pushed: "true" });
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("HTTP 502");
    expect(calls(result.state, "gh release create")).toEqual([]);
  });

  it("refuses the files of a dry run", () => {
    writeState(pushedTag);
    provenance({ dryRun: true });
    const result = run("upload_or_explain", { pushed: "true" });
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("made by a dry run");
    expect(calls(result.state, "gh ")).toEqual([]);
  });

  it("refuses files built from another commit than the tag on origin", () => {
    writeState({ remote: { main: OTHER, tag: OTHER } });
    provenance();
    const result = run("upload_or_explain", { pushed: "true" });
    expect(result.status).toBe(1);
    expect(result.stderr).toContain(`built from ${RELEASE}, but the tag points to ${OTHER}`);
    expect(calls(result.state, "gh ")).toEqual([]);
  });

  it("refuses files changed after the build", () => {
    writeState(pushedTag);
    provenance();
    writeFileSync(join(dir, "out", "latest.json"), "changed\n");
    const result = run("upload_or_explain", { pushed: "true" });
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("changed since the build: latest.json");
    expect(calls(result.state, "gh ")).toEqual([]);
  });

  it("refuses to upload when the tag is not on origin", () => {
    writeState();
    provenance();
    const result = run("upload_or_explain", { pushed: "true" });
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("tag v0.2.0 is not on origin");
    expect(calls(result.state, "gh ")).toEqual([]);
  });

  it("refuses files without provenance", () => {
    writeState(pushedTag);
    const result = run("upload_or_explain", { pushed: "true" });
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("provenance.json is missing");
    expect(calls(result.state, "gh ")).toEqual([]);
  });
});

describe("remote_refs", () => {
  it("reads main and the commit an annotated tag points to from a real origin", () => {
    const git = (cwd, ...args) => {
      const result = spawnSync(
        "git",
        ["-c", "user.name=t", "-c", "user.email=t@example.invalid", ...args],
        { cwd, encoding: "utf8" },
      );
      expect(result.status, result.stderr).toBe(0);
      return result.stdout.trim();
    };
    git(dir, "init", "-q", "--bare", "-b", "main", "origin.git");
    git(dir, "init", "-q", "-b", "main", "clone");
    const clone = join(dir, "clone");
    git(clone, "commit", "-q", "--allow-empty", "-m", "first");
    const commit = git(clone, "rev-parse", "HEAD");
    git(clone, "remote", "add", "origin", join(dir, "origin.git"));
    git(clone, "push", "-q", "origin", "main");
    // remote_refs and its time-limited form (used to judge a push) must agree.
    const read = () => {
      const [plain, timed] = ["remote_refs", "remote_refs_within"].map((fn) =>
        spawnSync("sh", ["-c", `. "${ROOT}/scripts/release-lib.sh"; tag=v0.2.0; ${fn}`], {
          cwd: clone,
          encoding: "utf8",
        }).stdout.trim(),
      );
      expect(timed).toBe(plain);
      return plain;
    };
    expect(read()).toBe(`${commit} -`);
    git(clone, "tag", "-a", "v0.2.0", "-m", "Woodshed 0.2.0");
    git(clone, "push", "-q", "origin", "refs/tags/v0.2.0");
    expect(git(clone, "rev-parse", "v0.2.0")).not.toBe(commit);
    expect(read()).toBe(`${commit} ${commit}`);
  });
});
