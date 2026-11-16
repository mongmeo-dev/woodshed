#!/usr/bin/env python3
"""Writes the third-party notices of everything compiled into the app:

- LICENSES/rust-crates.{md,txt}: every Rust crate linked into the app binary
  (normal dependencies of the `woodshed` crate on macOS, proc-macros left out
  because they only run at build time), read from the local Cargo registry.
- LICENSES/npm-packages.{md,txt}: every npm package bundled into the web view
  (`pnpm licenses list --prod`), read from node_modules.

Identical license texts are written once, under the list of packages they
cover. Run from the repository root after changing dependencies:
    mise exec -- python3 scripts/collect-licenses.py
    mise exec -- pnpm exec biome format --write LICENSES/rust-crates.md LICENSES/npm-packages.md
"""
import json
import os
import subprocess
import sys

TARGETS = ["aarch64-apple-darwin", "x86_64-apple-darwin"]
# Preference when a package offers a choice.
PREFERENCE = ["MIT", "0BSD", "Zlib", "Apache-2.0", "MPL-2.0"]
# Bundled C/C++ code inside a crate, noticed separately.
EMBEDDED = {
    "signalsmith-stretch": [
        ("Signalsmith Stretch 1.3.1 (C++)", "signalsmith-stretch/LICENSE.txt"),
        ("Signalsmith Linear (C++)", "signalsmith-linear/LICENSE.txt"),
    ],
}
# Crates published without a license file: the text from their repository.
UPSTREAM = {
    # rustaudio/dasp LICENSE-MIT.
    "dasp": "LICENSES/upstream/dasp-LICENSE-MIT.txt",
    # dropbox/rust-alloc-no-stdlib LICENSE at 0a81fd6928ea (the published commit).
    "alloc-stdlib": "LICENSES/upstream/alloc-stdlib-LICENSE.txt",
    # servo/stylo has no license file; every source file carries the MPL-2.0
    # notice pointing at https://mozilla.org/MPL/2.0/, whose text this is.
    "selectors": "LICENSES/upstream/MPL-2.0.txt",
}
# The objc2 crates publish no license file; the repository's MIT text
# (madsmtm/objc2 ee9a7ada2131, "Include license text in the repository").
OBJC2_FAMILY = ("objc2", "block2", "dispatch2")
OBJC2_TEXT = "LICENSES/upstream/objc2-LICENSE-MIT.txt"

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
out_dir = os.path.join(root, "LICENSES")
rule = "=" * 78


def choose(expression):
    """The license the app uses: one option of each `AND` term, e.g.
    `(MIT OR Apache-2.0) AND Unicode-3.0` → `MIT AND Unicode-3.0`."""
    chosen = []
    for term in expression.replace("(", "").replace(")", "").split(" AND "):
        options = [o.strip() for o in term.replace("/", " OR ").split(" OR ")]
        chosen.append(next((p for p in PREFERENCE if p in options), options[0]))
    return " AND ".join(chosen)


FILE_KEYS = {"MIT": "MIT", "Apache-2.0": "APACHE", "0BSD": "0BSD", "Zlib": "ZLIB",
             "BSD-3-Clause": "BSD", "Unicode-3.0": "UNICODE"}


def license_files(directory, chosen):
    files = sorted(
        os.path.join(directory, name)
        for name in os.listdir(directory)
        if name.upper().startswith(("LICENSE", "LICENCE", "COPYING"))
        and os.path.isfile(os.path.join(directory, name))
    )
    # Every file applies under `AND`; under a choice keep the chosen one's.
    if len(files) > 1 and " AND " not in chosen:
        wanted = [FILE_KEYS.get(chosen)]
        matching = [f for f in files if any(w and w in os.path.basename(f).upper() for w in wanted)]
        files = matching or files
    return files


def read(path):
    with open(path, encoding="utf-8", errors="replace") as handle:
        return "\n".join(line.rstrip() for line in handle.read().strip().split("\n"))


def write_texts(name, entries):
    """entries: (label, text). Identical texts are written once."""
    grouped = {}
    for label, text in entries:
        grouped.setdefault(text, []).append(label)
    blocks = [
        f"{rule}\n" + "\n".join(labels) + f"\n{rule}\n\n{text}\n"
        for text, labels in grouped.items()
    ]
    with open(os.path.join(out_dir, name), "w", encoding="utf-8") as handle:
        handle.write("\n".join(blocks))
    return len(grouped)


def table(rows):
    return "| Package | Version | License | Used | Source |\n|---|---|---|---|---|\n" + "\n".join(rows) + "\n"


# ---- Rust -------------------------------------------------------------------
manifest = os.path.join(root, "src-tauri", "Cargo.toml")
command = ["cargo", "metadata", "--format-version", "1", "--locked", "--manifest-path", manifest]
for target in TARGETS:
    command += ["--filter-platform", target]
meta = json.loads(subprocess.check_output(command))
packages = {p["id"]: p for p in meta["packages"]}
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}


def normal_deps(pid):
    return [
        dep["pkg"]
        for dep in nodes[pid]["deps"]
        if any(k["kind"] in (None, "normal") for k in dep["dep_kinds"])
    ]


closure = set()
stack = normal_deps(meta["resolve"]["root"])
while stack:
    pid = stack.pop()
    if pid in closure:
        continue
    package = packages[pid]
    if any(t["kind"] == ["proc-macro"] for t in package["targets"]):
        continue  # runs at build time only; not in the app
    closure.add(pid)
    stack.extend(normal_deps(pid))

rows = []
entries = []
missing = []
for pid in sorted(closure, key=lambda p: (packages[p]["name"], packages[p]["version"])):
    package = packages[pid]
    name, version = package["name"], package["version"]
    directory = os.path.dirname(package["manifest_path"])
    expression = package["license"] or ""
    chosen = choose(expression)
    files = license_files(directory, chosen)
    if not files:
        if name.startswith(OBJC2_FAMILY):
            files = [os.path.join(root, OBJC2_TEXT)]
        elif name.split("_")[0] in UPSTREAM:
            files = [os.path.join(root, UPSTREAM[name.split("_")[0]])]
        else:
            missing.append(f"{name} {version}")
    repo = package.get("repository") or f"https://crates.io/crates/{name}"
    rows.append(f"| {name} | {version} | {expression} | {chosen} | {repo} |")
    for path in files:
        entries.append((f"{name} {version} — {chosen}", read(path)))
    for title, relative in EMBEDDED.get(name, []):
        entries.append((f"{title}, bundled in {name} {version} — MIT", read(os.path.join(directory, relative))))

if missing:
    sys.exit("no license text for: " + ", ".join(missing) + " (add it to UPSTREAM)")

with open(os.path.join(out_dir, "rust-crates.md"), "w", encoding="utf-8") as handle:
    handle.write(
        "# Rust crates\n\n"
        "Every crate linked into the app binary on macOS (Tauri, the audio engine, decoding, time stretching, "
        "resampling, FFT, Core ML bindings and their dependencies). Generated by `scripts/collect-licenses.py`; "
        "the license texts are in [`rust-crates.txt`](rust-crates.txt). "
        "Where a crate offers a choice, the app uses it under the license in the \"Used\" column.\n\n"
        "The MPL-2.0 crates (Symphonia, `cssparser`, `selectors` and the others with MPL-2.0 in the \"Used\" column) "
        "are used unmodified; their source is at the repository in the \"Source\" column and on crates.io.\n\n"
    )
    handle.write(table(rows))
rust_texts = write_texts("rust-crates.txt", entries)
print(f"rust: {len(rows)} crates, {rust_texts} distinct texts", file=sys.stderr)

# ---- npm --------------------------------------------------------------------
listing = json.loads(subprocess.check_output(["pnpm", "licenses", "list", "--prod", "--json"], cwd=root))
rows = []
entries = []
for package in sorted((p for group in listing.values() for p in group), key=lambda p: p["name"]):
    expression = package["license"]
    chosen = choose(expression)
    for version, path in zip(package["versions"], package["paths"]):
        files = license_files(path, chosen)
        if not files:
            sys.exit(f"no license text for npm {package['name']} {version}")
        source = package.get("homepage") or f"https://www.npmjs.com/package/{package['name']}"
        rows.append(f"| {package['name']} | {version} | {expression} | {chosen} | {source} |")
        for file in files:
            entries.append((f"{package['name']} {version} — {chosen}", read(file)))

with open(os.path.join(out_dir, "npm-packages.md"), "w", encoding="utf-8") as handle:
    handle.write(
        "# npm packages\n\n"
        "Every npm package bundled into the app's web view (production dependencies). "
        "Generated by `scripts/collect-licenses.py`; the license texts are in "
        "[`npm-packages.txt`](npm-packages.txt).\n\n"
    )
    handle.write(table(rows))
npm_texts = write_texts("npm-packages.txt", entries)
print(f"npm: {len(rows)} packages, {npm_texts} distinct texts", file=sys.stderr)
