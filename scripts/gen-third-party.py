#!/usr/bin/env python3
"""Regenerate THIRD-PARTY.md from the runtime dependency graph."""
import collections
import json
import subprocess

runtime_tree = subprocess.run(
    ["cargo", "tree", "-e", "normal", "--prefix", "none", "--no-dedupe"],
    capture_output=True, text=True, check=True,
).stdout
metadata = json.loads(subprocess.run(
    ["cargo", "metadata", "--format-version", "1"],
    capture_output=True, text=True, check=True,
).stdout)

info = {
    (p["name"], "v" + p["version"]): (p.get("license") or "see license file", p.get("repository") or "")
    for p in metadata["packages"]
}

seen = set()
for line in runtime_tree.splitlines():
    parts = line.replace(" (*)", "").split()
    if len(parts) >= 2 and parts[1].startswith("v") and parts[0] != "vectorise":
        seen.add((parts[0], parts[1]))

by_license = collections.defaultdict(list)
for name, version in sorted(seen):
    license_name = info.get((name, version), ("?", ""))[0]
    by_license[license_name].append((name, version))

out = [
    "# Third-party notices",
    "",
    "`vectorise` is licensed MIT OR Apache-2.0. A **binary** of it also contains",
    "code from the crates below, whose licences carry obligations of their own.",
    "This file is that notice.",
    "",
    f"{len(seen)} crates are linked into a release binary. Test-only and build-only",
    "dependencies are excluded: they are not in what you download.",
    "",
    "Generated from `cargo tree -e normal` and `cargo metadata`. Regenerate with",
    "`just third-party`.",
    "",
    "## What each licence asks of you",
    "",
    "| Licence | Obligation when redistributing a binary |",
    "|---|---|",
    "| MIT, ISC, BSD-2-Clause, BSD-3-Clause, Zlib, Apache-2.0, 0BSD, Unlicense, BSL-1.0, Unicode-3.0 | reproduce the copyright notice and licence text. This file, shipped beside the binary, does that. |",
    '| MPL-2.0 | the MPL-covered **files** stay under MPL-2.0, and their source must be available to recipients. It does not reach our code or the rest of the binary (MPL-2.0 §3.3, "Larger Work"). Sources are public on crates.io and at the repositories listed below. |',
    "",
    "No crate in the graph is GPL, LGPL, or AGPL. `cargo deny check licenses`",
    "enforces the allow-list in `deny.toml` on every CI run.",
    "",
    "## MPL-2.0 crates, and where their source is",
    "",
    "| Crate | Version | Source |",
    "|---|---|---|",
]
for name, version in by_license.get("MPL-2.0", []):
    repository = info[(name, version)][1] or f"https://crates.io/crates/{name}"
    out.append(f"| {name} | {version[1:]} | {repository} |")

out += ["", "## Everything linked, by licence", ""]
for license_name in sorted(by_license, key=lambda key: (-len(by_license[key]), key)):
    crates = by_license[license_name]
    out += [
        f"### {license_name}",
        "",
        f"{len(crates)} crate(s): "
        + ", ".join(f"{name} {version[1:]}" for name, version in crates)
        + ".",
        "",
    ]

with open("THIRD-PARTY.md", "w", encoding="utf-8") as handle:
    handle.write("\n".join(out) + "\n")
print(f"THIRD-PARTY.md: {len(seen)} crates, {len(by_license)} distinct licence expressions")
