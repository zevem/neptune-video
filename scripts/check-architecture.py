#!/usr/bin/env python3
"""Small dependency-boundary guard; Rust privacy remains the primary contract."""

from __future__ import annotations

import argparse
from pathlib import Path
import re
import sys
import tomllib


PURE_CORE_DEPENDENCIES = {"serde", "serde_json", "thiserror", "uuid", "schemars"}
CORE_IO = re.compile(r"\bstd\s*::\s*(fs|process|thread|net)\b")


def dependencies(manifest: dict) -> set[str]:
    """Include platform-specific ordinary/build dependencies, excluding dev tools."""
    names: set[str] = set()
    sections = [manifest]
    sections.extend(manifest.get("target", {}).values())
    for section in sections:
        for kind in ("dependencies", "build-dependencies"):
            for alias, value in section.get(kind, {}).items():
                names.add(value.get("package", alias) if isinstance(value, dict) else alias)
    return names


def validate_graph(manifests: dict[str, dict]) -> list[str]:
    failures: list[str] = []
    expected = {"aster-editor", "editor-core", "editor-media", "editor-store", "editor-agent"}
    for name in sorted(expected - manifests.keys()):
        failures.append(f"missing package: {name}")
    core = manifests.get("editor-core", {})
    for name in sorted(dependencies(core) - PURE_CORE_DEPENDENCIES):
        failures.append(f"editor-core cannot depend on {name}")
    for package in ("editor-media", "editor-store", "editor-agent"):
        deps = dependencies(manifests.get(package, {}))
        if "editor-core" not in deps:
            failures.append(f"{package} must use editor-core's shared domain types")
        for name in sorted(deps & {"aster-editor", "editor-media", "editor-store", "editor-agent"}):
            failures.append(f"{package} cannot depend on {name}")
    desktop_deps = dependencies(manifests.get("aster-editor", {}))
    for name in sorted({"editor-core", "editor-media", "editor-store", "editor-agent"} - desktop_deps):
        failures.append(f"aster-editor must compose {name}")
    return failures


def inspect_repository(root: Path) -> list[str]:
    manifests: dict[str, dict] = {}
    failures: list[str] = []
    paths = [root / "Cargo.toml"]
    paths.extend(sorted((root / "crates").glob("*/Cargo.toml")))
    for path in paths:
        try:
            with path.open("rb") as source:
                manifest = tomllib.load(source)
            name = manifest.get("package", {}).get("name")
            if name:
                if name in manifests:
                    failures.append(f"duplicate package: {name}")
                manifests[name] = manifest
        except (OSError, tomllib.TOMLDecodeError) as error:
            failures.append(f"{path.relative_to(root)}: {error}")
    failures.extend(validate_graph(manifests))
    for path in sorted((root / "crates/editor-core/src").rglob("*.rs")):
        try:
            source = path.read_text(encoding="utf-8")
        except (OSError, UnicodeError) as error:
            failures.append(f"{path.relative_to(root)}: {error}")
            continue
        for line_number, line in enumerate(source.splitlines(), start=1):
            # This lightweight check reports direct source paths;
            # aliases/macros/transitive dependencies require review and Rust types.
            if line.lstrip().startswith("//"):
                continue
            if CORE_IO.search(line):
                failures.append(
                    f"{path.relative_to(root)}:{line_number}: core owns no filesystem/process/thread/network I/O"
                )
    return failures


def self_test() -> None:
    graph = {
        "aster-editor": {"dependencies": {"editor-core": {}, "editor-media": {}, "editor-store": {}, "editor-agent": {}}},
        "editor-core": {"dependencies": {"serde": {}, "uuid": {}}},
        "editor-media": {"dependencies": {"editor-core": {}}},
        "editor-store": {"dependencies": {"editor-core": {}}},
        "editor-agent": {"dependencies": {"editor-core": {}}},
    }
    assert not validate_graph(graph)
    graph["editor-core"]["target"] = {
        "cfg(windows)": {"dependencies": {"renamed": {"package": "eframe"}}}
    }
    assert "editor-core cannot depend on eframe" in validate_graph(graph)
    graph["editor-store"]["build-dependencies"] = {"editor-media": {}}
    assert "editor-store cannot depend on editor-media" in validate_graph(graph)
    graph["editor-agent"]["dependencies"]["editor-store"] = {}
    assert "editor-agent cannot depend on editor-store" in validate_graph(graph)
    assert CORE_IO.search("let source = std::fs::read(path);")
    assert not CORE_IO.search("use std::sync::Arc;")
    print("Architecture checker self-tests passed.")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    failures = inspect_repository(args.root.resolve())
    if failures:
        for failure in failures:
            print(f"ERROR: {failure}", file=sys.stderr)
        return 1
    print("Architecture boundaries passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
