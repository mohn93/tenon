#!/usr/bin/env python3
"""Licence notices of the Rust crates compiled into the shipped app.

  scripts/licenses.py            rewrite the generated section of THIRD_PARTY_NOTICES.md
  scripts/licenses.py --check    fail when that section is stale, or a crate has no licence
                                 expression, no licence text, or a licence not on ALLOWED

The crates are those reachable through normal (not dev or build) dependency edges from the `app`
crate on the native targets that are packaged and from the `web` crate on wasm32, as `cargo
metadata --filter-platform` resolves them from Cargo.lock. Each crate's licence files (LICENSE*,
COPYING*, NOTICE* ...) are read from its package in the cargo registry source cache; identical
texts are printed once with the crates that carry them. Python standard library only; `cargo`
must be on the PATH. With --offline cargo does not fetch (the packages must already be cached).
"""
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NOTICES = ROOT / "THIRD_PARTY_NOTICES.md"
BEGIN = "<!-- BEGIN GENERATED CRATE LICENCES (scripts/licenses.py; do not edit) -->"
END = "<!-- END GENERATED CRATE LICENCES -->"

# The targets of the packages (macOS universal, Windows, Linux) and of the browser build.
APP_TARGETS = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
]
WEB_TARGET = "wasm32-unknown-unknown"

# SPDX identifiers a compiled-in crate may be used under. A crate passes when its expression can
# be satisfied from this set (an OR needs one allowed branch, an AND needs every term).
#   MIT, Apache-2.0 (and its LLVM exception), BSD-2/3-Clause, ISC, Zlib, 0BSD, Unlicense, CC0-1.0,
#   BSL-1.0, OpenSSL: permissive; the only duty is to ship the copyright and licence text.
#   Unicode-3.0 and Unicode-DFS-2016: permissive data licences of the unicode tables.
#   CDLA-Permissive-2.0: the Mozilla root certificates in webpki-roots; permissive, text shipped.
#   OFL-1.1 and Ubuntu-font-1.0: the fonts built into the app; they allow bundling unmodified.
#   MPL-2.0: file-level copyleft; allowed because the crates are used unmodified from crates.io,
#   so only their own files carry the licence and the source is the published crate.
# Not allowed: GPL, LGPL, AGPL and any other licence not named here.
ALLOWED = {
    "MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-2-Clause", "BSD-3-Clause", "ISC",
    "Zlib", "0BSD", "Unlicense", "CC0-1.0", "BSL-1.0", "OpenSSL", "Unicode-3.0",
    "Unicode-DFS-2016", "CDLA-Permissive-2.0", "OFL-1.1", "Ubuntu-font-1.0", "MPL-2.0",
}

# Crates whose Cargo.toml expression leaves out part of what their licence file holds.
EXPRESSION_OVERRIDES = {
    # ring 0.17's package holds LICENSE-BoringSSL (Apache-2.0, for the code from BoringSSL, whose
    # OpenSSL-derived parts are Apache-2.0 since OpenSSL 3), LICENSE-other-bits (ISC, its own
    # code) and MIT texts for fiat-crypto and once_cell. No OpenSSL/SSLeay licence text is in it.
    "ring": "Apache-2.0 AND ISC AND MIT",
}

# Crates published without a licence file take these copies from packaging/licenses (taken from
# their upstream repositories; its README lists them). The first matching pattern wins.
FALLBACK_DIR = ROOT / "packaging" / "licenses"
FALLBACKS = [
    (r"accesskit_winit", ["accesskit-Apache-2.0.txt"]),
    (r"accesskit.*", ["accesskit-MIT.txt", "accesskit-Apache-2.0.txt"]),
    (r"dispatch", ["dispatch-MIT.txt"]),
    (r"block2|dispatch2|objc-sys|objc2.*", ["objc2-MIT.txt", "objc2-Apache-2.0.txt", "objc2-Zlib.txt"]),
    (r"ecolor|egui.*|emath|epaint.*", ["egui-MIT.txt", "egui-Apache-2.0.txt"]),
    (r"defmt-parser", ["defmt-MIT.txt", "defmt-Apache-2.0.txt"]),
    (r"profiling", ["profiling-MIT.txt", "profiling-Apache-2.0.txt"]),
    (r"clipboard-win", ["clipboard-win-BSL-1.0.txt"]),
    (r"spirv", ["rspirv-Apache-2.0.txt"]),
]

SKIP_DIRS = {"target", "tests", "test", "testdata", "examples", "benches", "fuzz", "node_modules"}

# File names taken for a licence or notice text.
LICENCE_FILE = re.compile(
    r"^(LICEN[CS]E|COPYING|COPYRIGHT|NOTICE|UNLICENSE|PATENTS)([-_.].*)?$", re.IGNORECASE
)


def metadata(target):
    cmd = ["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", target]
    if OFFLINE:
        cmd.append("--offline")
    out = subprocess.run(cmd, cwd=ROOT, check=True, capture_output=True, text=True).stdout
    return json.loads(out)


def shipped(meta, root_names):
    """Packages reachable from the named workspace crates through normal dependency edges."""
    packages = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    stack = [i for i, p in packages.items() if p["name"] in root_names and p["source"] is None]
    seen = set()
    while stack:
        i = stack.pop()
        if i in seen:
            continue
        seen.add(i)
        for dep in nodes[i]["deps"]:
            if any(k["kind"] is None for k in dep["dep_kinds"]):
                stack.append(dep["pkg"])
    return [packages[i] for i in seen]


def collect():
    found = {}
    for target in APP_TARGETS:
        for p in shipped(metadata(target), {"app"}):
            found[p["id"]] = p
    for p in shipped(metadata(WEB_TARGET), {"web"}):
        found[p["id"]] = p
    # The workspace's own crates are not third party.
    return sorted(
        (p for p in found.values() if p["source"] is not None or p["manifest_path"].startswith(str(ROOT / "vendor"))),
        key=lambda p: (p["name"], p["version"]),
    )


def licence_files(package):
    """The licence and notice files of a package, anywhere in it but its tests and examples
    (a crate that bundles third-party code, like ring, keeps their licences in subfolders)."""
    folder = Path(package["manifest_path"]).parent
    files = []
    for path in sorted(folder.rglob("*")):
        rel = path.relative_to(folder)
        if not path.is_file() or any(part.lower() in SKIP_DIRS for part in rel.parts[:-1]):
            continue
        if LICENCE_FILE.match(path.name):
            files.append(path)
    return files


def fallback_files(name):
    for pattern, names in FALLBACKS:
        if re.fullmatch(pattern, name):
            return [FALLBACK_DIR / n for n in names]
    return []


def normal(text):
    return text.replace("\r\n", "\n").replace("\r", "\n").strip("\n").rstrip()


def tokens(expr):
    expr = expr.replace("/", " OR ")
    return re.findall(r"\(|\)|[A-Za-z0-9.+-]+", expr)


def satisfied(expr, allowed=ALLOWED):
    """True when the SPDX expression can be met using only allowed identifiers."""
    toks = tokens(expr)
    pos = 0

    def peek():
        return toks[pos] if pos < len(toks) else None

    def take():
        nonlocal pos
        pos += 1
        return toks[pos - 1]

    def term():
        if peek() == "(":
            take()
            v = orexpr()
            if peek() != ")":
                raise ValueError(f"bad expression {expr!r}")
            take()
            return v
        name = take()
        if peek() == "WITH":
            take()
            name = f"{name} WITH {take()}"
        return name in allowed

    def andexpr():
        v = term()
        while peek() == "AND":
            take()
            v = term() and v
        return v

    def orexpr():
        v = andexpr()
        while peek() == "OR":
            take()
            v = andexpr() or v
        return v

    v = orexpr()
    if pos != len(toks):
        raise ValueError(f"bad expression {expr!r}")
    return v


def fence_for(text):
    longest = max((len(m) for m in re.findall(r"`+", text)), default=0)
    return "`" * max(3, longest + 1)


def generate(packages):
    problems = []
    groups = {}  # text -> [crate labels]
    lines = []
    for p in packages:
        label = f"{p['name']} {p['version']}"
        expr = EXPRESSION_OVERRIDES.get(p["name"]) or p["license"]
        if not expr:
            problems.append(f"{label}: no licence expression")
            continue
        try:
            ok = satisfied(expr)
        except ValueError as e:
            problems.append(f"{label}: {e}")
            continue
        if not ok:
            problems.append(f"{label}: licence {expr!r} is not on the allowed list")
        files = licence_files(p) or fallback_files(p["name"])
        if not files:
            problems.append(f"{label}: no licence file in the package")
        for f in files:
            text = normal(f.read_bytes().decode("utf-8", errors="replace"))
            if text:
                groups.setdefault(text, []).append(label)
        lines.append(f"- {label}: {expr}")

    out = [
        BEGIN,
        "",
        "## Rust crates compiled into the app",
        "",
        "These crates (from `Cargo.lock`) are compiled into the desktop app and the browser build,",
        "each under the licence named after it. The licence and notice files each crate ships",
        "follow; identical texts are printed once, with the crates that carry them.",
        "",
        f"{len(lines)} crates:",
        "",
        *lines,
        "",
        "### Licence texts",
        "",
    ]
    ordered = sorted(groups.items(), key=lambda kv: (-len(kv[1]), kv[0]))
    for n, (text, crates) in enumerate(ordered, 1):
        fence = fence_for(text)
        out += [f"#### Text {n}", "", "Carried by: " + ", ".join(sorted(set(crates))), "", fence, text, fence, ""]
    out.append(END)
    return "\n".join(out) + "\n", problems


def splice(current, section):
    if BEGIN in current and END in current:
        head = current[: current.index(BEGIN)]
        tail = current[current.index(END) + len(END):].lstrip("\n")
        return head + section + ("\n" + tail if tail else "")
    return current.rstrip("\n") + "\n\n" + section


OFFLINE = "--offline" in sys.argv


def main():
    check = "--check" in sys.argv
    section, problems = generate(collect())
    for p in problems:
        print("licences:", p, file=sys.stderr)
    if problems:
        return 1
    current = NOTICES.read_text()
    updated = splice(current, section)
    if check:
        if updated != current:
            print("THIRD_PARTY_NOTICES.md is stale: run scripts/licenses.py and commit it", file=sys.stderr)
            return 1
        print("licences: ok")
        return 0
    NOTICES.write_text(updated)
    print(f"wrote the crate licences to {NOTICES.name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
