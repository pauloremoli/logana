#!/usr/bin/env python3
"""Verify frontend isolation in Cargo's normal/build dependency graphs."""
import json
import subprocess

TERMINAL = {"ratatui", "ratatui-core", "ratatui-crossterm", "crossterm", "arboard"}
GUI = {"gpui-kit", "gpui-component", "gpui-base", "gpui-pre"}

for label, package, features, forbidden in [
    ("core", "logana", [], TERMINAL | GUI),
    ("tui", "logana", ["tui"], GUI),
    ("gui", "logana", ["gui"], TERMINAL),
    ("logana-gui package", "logana-gui", None, TERMINAL),
]:
    command = ["cargo", "tree", "--locked", "-p", package,
               "--edges", "normal,build", "--prefix", "none", "--format", "{p}"]
    if features is not None:
        command.append("--no-default-features")
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
packages_by_name = {p["name"]: p for p in json.loads(metadata)["packages"]}
for package_name, target, feature in [
    ("logana", "logana", "tui"),
    ("logana-full", "logana", None),
    ("logana-full", "logana-gui", None),
    ("logana-full", "schema", None),
    ("logana-gui", "logana-gui", None),
]:
    targets = packages_by_name[package_name]["targets"]
    entry = next(t for t in targets if t["name"] == target and t["kind"] == ["bin"])
    expected = [feature] if feature else []
    assert entry.get("required-features", []) == expected, (package_name, target, entry)
print("frontend binaries require their respective features")
