// @vitest-environment node
// Fixtures: scripts/fixtures/update-archive.txt signed by `tauri signer sign`
// with a throwaway key (update-test.pub; the private half was deleted), and
// the public key of another throwaway key (other-key.pub).
import { readFileSync } from "node:fs";
import {
  assetsProblem,
  bumpCargoLock,
  bumpCargoToml,
  bumpJson,
  checkNextVersion,
  compareVersions,
  latestJson,
  newerRelease,
  parseVersion,
  provenance,
  provenanceProblem,
  readVersions,
  signatureProblem,
  signedMessage,
} from "./release-files.mjs";

const fixture = (name) => new URL(`./fixtures/${name}`, import.meta.url);
const read = (name) => readFileSync(fixture(name), "utf8");

describe("versions", () => {
  it("accepts only X.Y.Z", () => {
    expect(parseVersion("0.2.0")).toEqual([0, 2, 0]);
    for (const bad of ["v0.2.0", "0.2", "0.2.0-beta.1", "01.2.0", "0.2.0+1", ""]) {
      expect(() => parseVersion(bad)).toThrow();
    }
  });

  it("compares numerically", () => {
    expect(compareVersions("0.10.0", "0.9.9")).toBe(1);
    expect(compareVersions("1.0.0", "1.0.0")).toBe(0);
    expect(compareVersions("0.2.0", "0.2.1")).toBe(-1);
  });

  it("requires a version above the current one and every tag", () => {
    expect(() => checkNextVersion("0.2.0", "0.1.0", ["v0.1.0"])).not.toThrow();
    expect(() => checkNextVersion("0.1.0", "0.1.0")).toThrow(/not above the current/);
    expect(() => checkNextVersion("0.2.0", "0.1.0", ["v0.2.0"])).toThrow(/existing tag v0.2.0/);
    expect(() => checkNextVersion("0.2.0", "0.1.0", ["v0.3.1"])).toThrow(/existing tag/);
  });
});

describe("version bump", () => {
  it("changes only the top-level version of a JSON file", () => {
    const text =
      '{\n  "name": "x",\n  "version": "0.1.0",\n  "deps": {\n    "version": "9"\n  }\n}\n';
    expect(bumpJson(text, "0.2.0", "x")).toBe(text.replace('"0.1.0"', '"0.2.0"'));
    expect(() => bumpJson('{\n  "name": "x"\n}\n', "0.2.0", "x")).toThrow(/found 0/);
  });

  it("changes the [package] version of Cargo.toml only", () => {
    const text =
      '[workspace]\nmembers = ["."]\n\n[package]\nname = "woodshed"\nversion = "0.1.0"\n\n[dependencies]\nfoo = { version = "1" }\n';
    expect(bumpCargoToml(text, "0.2.0")).toBe(text.replace('"0.1.0"', '"0.2.0"'));
  });

  it("changes the woodshed entry of Cargo.lock only", () => {
    const text =
      '[[package]]\nname = "woodpecker"\nversion = "0.1.0"\n\n[[package]]\nname = "woodshed"\nversion = "0.1.0"\ndependencies = [\n "x",\n]\n';
    const bumped = bumpCargoLock(text, "0.2.0");
    expect(bumped).toContain('name = "woodpecker"\nversion = "0.1.0"');
    expect(bumped).toContain('name = "woodshed"\nversion = "0.2.0"');
  });

  it("reads the same version from the repository's four files", () => {
    const versions = readVersions((path) =>
      readFileSync(new URL(`../${path}`, import.meta.url), "utf8"),
    );
    expect(new Set(Object.values(versions)).size).toBe(1);
    expect(Object.values(versions)[0]).toMatch(/^\d+\.\d+\.\d+$/);
  });
});

describe("update signature", () => {
  const signature = read("update-archive.txt.sig");
  const publicKey = read("update-test.pub");

  it("accepts the file signed by tauri signer", async () => {
    const message = await signedMessage(signature, fixture("update-archive.txt"));
    expect(signatureProblem(publicKey, signature, message)).toBeNull();
  });

  it("refuses a changed file", async () => {
    const message = Buffer.from(await signedMessage(signature, fixture("update-archive.txt")));
    message[0] ^= 1;
    expect(signatureProblem(publicKey, signature, message)).toMatch(/does not match the file/);
  });

  it("refuses another key", async () => {
    const message = await signedMessage(signature, fixture("update-archive.txt"));
    expect(signatureProblem(read("other-key.pub"), signature, message)).toMatch(/another key/);
  });

  it("refuses a changed trusted comment", async () => {
    const message = await signedMessage(signature, fixture("update-archive.txt"));
    const text = Buffer.from(signature, "base64")
      .toString("utf8")
      .replace("file:update-archive.txt", "file:other.txt");
    const forged = Buffer.from(text, "utf8").toString("base64");
    expect(signatureProblem(publicKey, forged, message)).toMatch(/trusted comment/);
  });
});

describe("latest.json", () => {
  it("has the fields the updater reads", () => {
    expect(
      latestJson({
        version: "0.2.0",
        notes: "Woodshed 0.2.0",
        pubDate: "2026-11-16T00:00:00Z",
        signature: "c2ln",
        url: "https://example.invalid/Woodshed.app.tar.gz",
      }),
    ).toEqual({
      version: "0.2.0",
      notes: "Woodshed 0.2.0",
      pub_date: "2026-11-16T00:00:00Z",
      platforms: {
        "darwin-aarch64": { signature: "c2ln", url: "https://example.invalid/Woodshed.app.tar.gz" },
      },
    });
  });
});

describe("release assets", () => {
  const hashes = { "a.dmg": "aa", "latest.json": "bb" };

  it("match only when every file is there with the same SHA-256 and nothing else", () => {
    const same = [
      { name: "a.dmg", digest: "sha256:aa" },
      { name: "latest.json", digest: "sha256:bb" },
    ];
    expect(assetsProblem(same, hashes)).toBeNull();
    expect(assetsProblem(same.slice(0, 1), hashes)).toBe("latest.json is missing");
    expect(assetsProblem([same[0], { name: "latest.json", digest: "sha256:cc" }], hashes)).toBe(
      "latest.json differs",
    );
    expect(assetsProblem([same[0], { name: "latest.json" }], hashes)).toBe(
      "latest.json has no digest",
    );
    expect(assetsProblem([...same, { name: "extra.zip", digest: "sha256:dd" }], hashes)).toBe(
      "extra.zip is not one of the release files",
    );
  });

  it("finds a published release above the version, ignoring drafts and prereleases", () => {
    const release = (tagName, more = {}) => ({
      tagName,
      isDraft: false,
      isPrerelease: false,
      ...more,
    });
    expect(newerRelease("0.2.0", [release("v0.1.0"), release("v0.2.0")])).toBeNull();
    expect(newerRelease("0.2.0", [release("v0.1.0"), release("v0.10.0")])).toBe("v0.10.0");
    expect(newerRelease("0.2.0", [release("v0.3.0", { isDraft: true })])).toBeNull();
    expect(newerRelease("0.2.0", [release("v0.3.0", { isPrerelease: true })])).toBeNull();
    expect(newerRelease("0.2.0", [release("nightly")])).toBeNull();
  });
});

describe("provenance", () => {
  const hashes = { "a.dmg": "aa", "latest.json": "bb" };
  const built = provenance({ version: "0.2.0", commit: "c1", dryRun: false, hashes });

  it("allows the release build of the tagged commit", () => {
    expect(built).toEqual({
      version: "0.2.0",
      tag: "v0.2.0",
      commit: "c1",
      dryRun: false,
      assets: hashes,
    });
    expect(provenanceProblem(built, { version: "0.2.0", commit: "c1", hashes })).toBeNull();
  });

  it("refuses a dry run, another commit or version, and changed or other files", () => {
    const check = (recorded, more = {}) =>
      provenanceProblem(recorded, { version: "0.2.0", commit: "c1", hashes, ...more });
    expect(check({ ...built, dryRun: true })).toMatch(/dry run/);
    expect(check(built, { commit: "c2" })).toMatch(/built from c1, but the tag points to c2/);
    expect(check({ ...built, version: "0.1.0" })).toMatch(/version 0.1.0/);
    expect(check(built, { hashes: { ...hashes, "latest.json": "zz" } })).toMatch(
      /changed since the build: latest.json/,
    );
    expect(check(built, { hashes: { "a.dmg": "aa" } })).toMatch(/other files/);
  });
});
