#!/usr/bin/env python3
"""Removes stale build output from target/debug/deps, keeping what the current build uses.

Cargo never deletes old artifacts. A deps folder with hundreds of thousands of files slows every
GPU test on macOS (Metal reads the test program's folder at start-up; see docs/testing.md). This
asks cargo which artifacts the current test, clippy and build runs use and deletes every other
`name-hash` file and folder in target/debug/deps. Nothing current is rebuilt afterwards.

Usage: python3 scripts/prune-target.py
"""
import json
import os
import re
import shutil
import subprocess

DEPS = "target/debug/deps"
NAME = re.compile(r"(?:lib)?(.+?-[0-9a-f]{16})")


def current():
    keep = set()
    for args in (
        ["test", "--workspace", "--lib", "--bins", "--tests", "--no-run"],
        ["clippy", "--workspace", "--all-targets"],
        ["build", "--workspace"],
    ):
        out = subprocess.run(
            ["cargo", *args, "--message-format=json"],
            capture_output=True, text=True, check=True,
        ).stdout
        for line in out.splitlines():
            try:
                m = json.loads(line)
            except ValueError:
                continue
            if m.get("reason") != "compiler-artifact":
                continue
            files = m.get("filenames", []) + ([m["executable"]] if m.get("executable") else [])
            for f in files:
                n = NAME.match(os.path.basename(f))
                if n:
                    keep.add(n.group(1))
    return keep


def main():
    keep = current()
    gone = 0
    for e in os.scandir(DEPS):
        n = NAME.match(e.name)
        if not n or n.group(1) in keep:
            continue
        if e.is_dir(follow_symlinks=False):
            shutil.rmtree(e.path, ignore_errors=True)
        else:
            os.unlink(e.path)
        gone += 1
    print(f"removed {gone} stale entries from {DEPS}")


if __name__ == "__main__":
    main()
