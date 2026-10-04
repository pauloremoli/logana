#!/usr/bin/env python3
"""Verify frontend isolation in Cargo's normal/build dependency graphs."""
import json
import subprocess

TERMINAL = {"ratatui", "ratatui-core", "ratatui-crossterm", "crossterm", "arboard"}
GUI = {"gpui-kit", "gpui-component", "gpui-base", "gpui-pre"}

for label, features, forbidden in [
    ("core", [], TERMINAL | GUI),
    ("tui", ["tui"], GUI),
    ("gui", ["gui"], TERMINAL),
]:
    command = ["cargo", "tree", "--locked", "--no-default-features",
               "--edges", "normal,build", "--prefix", "none", "--format", "{p}"]
    if features:
        command.extend(["--features", ",".join(features)])
    graph = subprocess.check_output(command, text=True)
    packages = {line.split()[0] for line in graph.splitlines() if line.strip()}
    unexpected = sorted(packages & forbidden)
    if unexpected:
        raise SystemExit(f"{label} unexpectedly includes: {', '.join(unexpected)}")
    print(f"{label}: frontend dependencies isolated")

metadata = subprocess.check_output(
    ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], text=True
)
package = next(p for p in json.loads(metadata)["packages"] if p["name"] == "logana")
for target, feature in [("logana", "tui"), ("logana-gui", "gui")]:
    entry = next(t for t in package["targets"] if t["name"] == target and t["kind"] == ["bin"])
    assert entry.get("required-features") == [feature], (target, entry)
print("frontend binaries require their respective features")
