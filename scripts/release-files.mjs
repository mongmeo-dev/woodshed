// File work for scripts/release.sh: version checks, the version bump in the
// four files that carry it, latest.json, and checking an update signature
// against the public key in tauri.conf.json (plugins.updater.pubkey).
//
//   node scripts/release-files.mjs current
//   node scripts/release-files.mjs valid <version>
//   node scripts/release-files.mjs check-version <new> [<existing tag>...]
//   node scripts/release-files.mjs bump <version>
//   node scripts/release-files.mjs latest-json <out> <version> <notes file> <sig file> <url>
//   node scripts/release-files.mjs verify-latest <latest.json> <archive> <version> <url>
//   node scripts/release-files.mjs provenance-write <dir> <version> <commit> <dry run: true|false> <file>...
//   node scripts/release-files.mjs provenance-check <dir> <version> <commit> <file>...
//   node scripts/release-files.mjs release-state <gh release view --json isDraft,assets>
//   node scripts/release-files.mjs assets-match <gh release view --json isDraft,assets> <dir> <file>...
//   node scripts/release-files.mjs check-newest <version> <gh release list --json tagName,isDraft,isPrerelease>
//
// Update signatures are minisign signatures (`tauri signer`), base64-encoded
// as a whole file, like the public key. They are checked here with node:crypto
// alone (Ed25519 over the BLAKE2b-512 hash of the file, plus the global
// signature over the trusted comment), so the check does not trust the tool
// that made them.
import { createHash, createPublicKey, verify } from "node:crypto";
import { createReadStream, existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const VERSION_RE = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

/** "1.2.3" → [1, 2, 3]; anything else (prerelease, build, leading v) throws. */
export function parseVersion(version) {
  const match = VERSION_RE.exec(version);
  if (!match) {
    throw new Error(`not a release version (X.Y.Z): ${version}`);
  }
  return match.slice(1).map(Number);
}

export function compareVersions(a, b) {
  const [x, y] = [parseVersion(a), parseVersion(b)];
  for (let i = 0; i < 3; i += 1) {
    if (x[i] !== y[i]) return x[i] < y[i] ? -1 : 1;
  }
  return 0;
}

/** Throws unless `next` is a release version above `current` and every tag vX.Y.Z. */
export function checkNextVersion(next, current, tags = []) {
  parseVersion(next);
  if (compareVersions(next, current) <= 0) {
    throw new Error(`${next} is not above the current version ${current}`);
  }
  for (const tag of tags) {
    const version = tag.replace(/^v/, "");
    if (VERSION_RE.test(version) && compareVersions(next, version) <= 0) {
      throw new Error(`${next} is not above the existing tag ${tag}`);
    }
  }
}

function replaceOnce(text, pattern, replacement, what) {
  const matches = text.match(new RegExp(pattern.source, `${pattern.flags.replace("g", "")}g`));
  if (matches?.length !== 1) {
    throw new Error(`${what}: expected one version field, found ${matches?.length ?? 0}`);
  }
  return text.replace(pattern, replacement);
}

/** package.json, tauri.conf.json: the top-level "version" (two-space indent). */
export function bumpJson(text, version, what) {
  return replaceOnce(text, /^ {2}"version": "[^"]*"/m, `  "version": "${version}"`, what);
}

/** src-tauri/Cargo.toml: version in [package]. */
export function bumpCargoToml(text, version) {
  return replaceOnce(
    text,
    /^(\[package\]\n(?:(?!\[)[^\n]*\n)*?)version = "[^"]*"/m,
    `$1version = "${version}"`,
    "Cargo.toml",
  );
}

/** src-tauri/Cargo.lock: the woodshed package entry. */
export function bumpCargoLock(text, version) {
  return replaceOnce(
    text,
    /^(\[\[package\]\]\nname = "woodshed"\n)version = "[^"]*"/m,
    `$1version = "${version}"`,
    "Cargo.lock",
  );
}

const VERSION_FILES = [
  ["package.json", (text, v) => bumpJson(text, v, "package.json")],
  ["src-tauri/tauri.conf.json", (text, v) => bumpJson(text, v, "tauri.conf.json")],
  ["src-tauri/Cargo.toml", bumpCargoToml],
  ["src-tauri/Cargo.lock", bumpCargoLock],
];

/** The version of each file that carries it (they must agree before a release). */
export function readVersions(read = (path) => readFileSync(path, "utf8")) {
  const versions = {};
  for (const [path] of VERSION_FILES) {
    const text = read(path);
    const match =
      path === "src-tauri/Cargo.lock"
        ? /^\[\[package\]\]\nname = "woodshed"\nversion = "([^"]*)"/m.exec(text)
        : path === "src-tauri/Cargo.toml"
          ? /^\[package\]\n(?:(?!\[)[^\n]*\n)*?version = "([^"]*)"/m.exec(text)
          : /^ {2}"version": "([^"]*)"/m.exec(text);
    versions[path] = match ? match[1] : null;
  }
  return versions;
}

export function latestJson({ version, notes, pubDate, signature, url }) {
  return {
    version,
    notes,
    pub_date: pubDate,
    platforms: { "darwin-aarch64": { signature, url } },
  };
}

function decodeMinisignFile(base64, what) {
  const lines = Buffer.from(base64.trim(), "base64").toString("utf8").split("\n");
  if (!lines[0]?.startsWith("untrusted comment:") || !lines[1]) {
    throw new Error(`${what} is not a minisign file`);
  }
  return lines;
}

/** Parses a tauri updater public key (base64 of a minisign public key file). */
export function parsePublicKey(base64) {
  const raw = Buffer.from(decodeMinisignFile(base64, "public key")[1], "base64");
  if (raw.length !== 42 || raw.subarray(0, 2).toString("latin1") !== "Ed") {
    throw new Error("public key is not a minisign Ed25519 key");
  }
  return { keyId: raw.subarray(2, 10), key: raw.subarray(10) };
}

/** Parses a tauri update signature (base64 of a minisign signature file). */
export function parseSignature(base64) {
  const lines = decodeMinisignFile(base64, "signature");
  const raw = Buffer.from(lines[1], "base64");
  const algorithm = raw.subarray(0, 2).toString("latin1");
  if (raw.length !== 74 || (algorithm !== "ED" && algorithm !== "Ed")) {
    throw new Error("signature is not a minisign Ed25519 signature");
  }
  const prefix = "trusted comment: ";
  if (!lines[2]?.startsWith(prefix) || !lines[3]) {
    throw new Error("signature has no trusted comment");
  }
  return {
    prehashed: algorithm === "ED",
    keyId: raw.subarray(2, 10),
    signature: raw.subarray(10),
    trustedComment: lines[2].slice(prefix.length),
    globalSignature: Buffer.from(lines[3], "base64"),
  };
}

function ed25519Key(raw) {
  return createPublicKey({
    key: { kty: "OKP", crv: "Ed25519", x: Buffer.from(raw).toString("base64url") },
    format: "jwk",
  });
}

/**
 * Checks a signature over `message`: the BLAKE2b-512 hash of the file for
 * prehashed ("ED") signatures, the file itself for legacy ("Ed") ones.
 * Returns null when it holds, otherwise the reason.
 */
export function signatureProblem(publicKeyBase64, signatureBase64, message) {
  const publicKey = parsePublicKey(publicKeyBase64);
  const sig = parseSignature(signatureBase64);
  if (!publicKey.keyId.equals(sig.keyId)) {
    return "signed with another key (key id differs)";
  }
  const key = ed25519Key(publicKey.key);
  if (!verify(null, message, key, sig.signature)) {
    return "signature does not match the file";
  }
  const global = Buffer.concat([sig.signature, Buffer.from(sig.trustedComment, "utf8")]);
  if (!verify(null, global, key, sig.globalSignature)) {
    return "trusted comment signature does not match";
  }
  return null;
}

/** What `signatureProblem` signs for `path`: its BLAKE2b-512 hash or its bytes. */
export async function signedMessage(signatureBase64, path) {
  if (!parseSignature(signatureBase64).prehashed) {
    return readFileSync(path);
  }
  const hash = createHash("blake2b512");
  for await (const chunk of createReadStream(path)) {
    hash.update(chunk);
  }
  return hash.digest();
}

function updaterPublicKey() {
  const conf = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
  const key = conf.plugins?.updater?.pubkey;
  if (!key) throw new Error("tauri.conf.json has no plugins.updater.pubkey");
  return key;
}

async function verifyLatest(latestPath, archive, version, url) {
  const latest = JSON.parse(readFileSync(latestPath, "utf8"));
  const platform = latest.platforms?.["darwin-aarch64"];
  const problems = [];
  if (latest.version !== version) problems.push(`version ${latest.version}, expected ${version}`);
  if (!platform) problems.push("no platforms.darwin-aarch64");
  if (platform && platform.url !== url) problems.push(`url ${platform.url}, expected ${url}`);
  if (Number.isNaN(Date.parse(latest.pub_date ?? ""))) problems.push("pub_date is not a date");
  if (platform) {
    const pubkey = updaterPublicKey();
    const message = await signedMessage(platform.signature, archive);
    const problem = signatureProblem(pubkey, platform.signature, message);
    if (problem) problems.push(problem);
    // The check must be able to fail: a changed byte has to be refused.
    const altered = Buffer.from(message);
    altered[0] ^= 1;
    if (!signatureProblem(pubkey, platform.signature, altered)) {
      problems.push("a changed file was accepted");
    }
  }
  if (problems.length > 0) {
    throw new Error(`${latestPath}: ${problems.join("; ")}`);
  }
  console.log(
    `latest.json ${version}: signature of ${archive} verified with the updater public key`,
  );
}

export async function sha256File(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) {
    hash.update(chunk);
  }
  return hash.digest("hex");
}

async function fileHashes(dir, names) {
  const hashes = {};
  for (const name of names) {
    hashes[name] = await sha256File(join(dir, name));
  }
  return hashes;
}

// Provenance (releases/vX.Y.Z/provenance.json, not uploaded): which commit
// the files were built from and whether a dry run made them, so an upload
// never sends files of another commit or of a dry run.
export const PROVENANCE = "provenance.json";

export function provenance({ version, commit, dryRun, hashes }) {
  return { version, tag: `v${version}`, commit, dryRun, assets: hashes };
}

/** Why the recorded build may not be uploaded for `commit`, or null. */
export function provenanceProblem(recorded, { version, commit, hashes }) {
  if (recorded.dryRun !== false) return "the files were made by a dry run";
  if (recorded.version !== version) return `the files are for version ${recorded.version}`;
  if (recorded.commit !== commit) {
    return `the files were built from ${recorded.commit}, but the tag points to ${commit}`;
  }
  const names = Object.keys(hashes);
  const recordedNames = Object.keys(recorded.assets ?? {});
  if (recordedNames.length !== names.length || recordedNames.some((name) => !(name in hashes))) {
    return `the build recorded other files (${recordedNames.join(", ")})`;
  }
  const changed = names.filter((name) => recorded.assets[name] !== hashes[name]);
  return changed.length > 0 ? `changed since the build: ${changed.join(", ")}` : null;
}

/** `gh release view --json isDraft,assets` → "draft" or "published". */
export function releaseState(view) {
  return view.isDraft ? "draft" : "published";
}

/**
 * Why the release's assets (`gh release view --json assets`: name, digest
 * "sha256:<hex>") are not exactly `hashes` (name → sha256 hex), or null.
 */
export function assetsProblem(assets, hashes) {
  const remote = new Map(assets.map((asset) => [asset.name, asset]));
  const problems = [];
  for (const [name, hash] of Object.entries(hashes)) {
    const asset = remote.get(name);
    if (!asset) problems.push(`${name} is missing`);
    else if (!asset.digest) problems.push(`${name} has no digest`);
    else if (asset.digest !== `sha256:${hash}`) problems.push(`${name} differs`);
  }
  for (const name of remote.keys()) {
    if (!(name in hashes)) problems.push(`${name} is not one of the release files`);
  }
  return problems.length > 0 ? problems.join(", ") : null;
}

/** The tag of a published release above `version` (drafts, prereleases left out), or null. */
export function newerRelease(version, releases) {
  const newer = releases.find((release) => {
    const other = release.tagName.replace(/^v/, "");
    return (
      !release.isDraft &&
      !release.isPrerelease &&
      VERSION_RE.test(other) &&
      compareVersions(other, version) > 0
    );
  });
  return newer ? newer.tagName : null;
}

const readJson = (path) => JSON.parse(readFileSync(path, "utf8"));

async function main([command, ...args]) {
  switch (command) {
    case "current": {
      const versions = readVersions();
      const distinct = new Set(Object.values(versions));
      if (distinct.size !== 1 || distinct.has(null)) {
        throw new Error(`version files disagree: ${JSON.stringify(versions)}`);
      }
      console.log([...distinct][0]);
      return;
    }
    case "valid": {
      parseVersion(args[0] ?? "");
      return;
    }
    case "check-version": {
      const [next, ...tags] = args;
      checkNextVersion(next, readVersions()["package.json"], tags);
      return;
    }
    case "bump": {
      const [version] = args;
      parseVersion(version);
      const updated = VERSION_FILES.map(([path, bump]) => [
        path,
        bump(readFileSync(path, "utf8"), version),
      ]);
      for (const [path, text] of updated) writeFileSync(path, text);
      return;
    }
    case "latest-json": {
      const [out, version, notesFile, sigFile, url] = args;
      const json = latestJson({
        version,
        notes: readFileSync(notesFile, "utf8").trim(),
        pubDate: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
        signature: readFileSync(sigFile, "utf8").trim(),
        url,
      });
      writeFileSync(out, `${JSON.stringify(json, null, 2)}\n`);
      return;
    }
    case "verify-latest": {
      const [latestPath, archive, version, url] = args;
      await verifyLatest(latestPath, archive, version, url);
      return;
    }
    case "provenance-write": {
      const [dir, version, commit, dryRun, ...names] = args;
      if (dryRun !== "true" && dryRun !== "false") throw new Error(`dry run: ${dryRun}`);
      const record = provenance({
        version,
        commit,
        dryRun: dryRun === "true",
        hashes: await fileHashes(dir, names),
      });
      writeFileSync(join(dir, PROVENANCE), `${JSON.stringify(record, null, 2)}\n`);
      return;
    }
    case "provenance-check": {
      const [dir, version, commit, ...names] = args;
      const path = join(dir, PROVENANCE);
      if (!existsSync(path)) throw new Error(`${path} is missing; build the release again`);
      const problem = provenanceProblem(readJson(path), {
        version,
        commit,
        hashes: await fileHashes(dir, names),
      });
      if (problem) throw new Error(`${dir}: ${problem}`);
      console.log(`${dir}: built from ${commit} (v${version}), not a dry run`);
      return;
    }
    case "release-state": {
      console.log(releaseState(readJson(args[0])));
      return;
    }
    case "assets-match": {
      const [viewPath, dir, ...names] = args;
      const problem = assetsProblem(readJson(viewPath).assets ?? [], await fileHashes(dir, names));
      if (problem) throw new Error(`release assets differ from ${dir}: ${problem}`);
      return;
    }
    case "check-newest": {
      const [version, listPath] = args;
      const newer = newerRelease(version, readJson(listPath));
      if (newer) {
        throw new Error(`${newer} is already published; ${version} would replace it as latest`);
      }
      return;
    }
    default:
      throw new Error(`unknown command: ${command ?? "(none)"}`);
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  main(process.argv.slice(2)).catch((error) => {
    console.error(`release-files: ${error.message}`);
    process.exit(1);
  });
}
