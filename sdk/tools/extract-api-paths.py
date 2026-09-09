#!/usr/bin/env python3
"""Regenerate sdk/api-paths.json from the Rust REST server.

The served /openapi.json comes from the hand-maintained `ApiDoc` derive in
`src-tauri/src/api_server.rs`, not from the axum router, so this script reads
the same two things the document is built from:

  * every `#[utoipa::path(...)]` annotation (its verb and path), and
  * the `paths(...)` list inside `#[openapi(...)]`.

An annotation that is not in `paths(...)` never reaches the served document, so
the two lists are compared here and a difference fails the run. The result is a
snapshot both SDK test suites read to prove they cover the whole API.

Usage (from anywhere):
    python3 sdk/tools/extract-api-paths.py
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SOURCE = REPO_ROOT / "src-tauri" / "src" / "api_server.rs"
SNAPSHOT = REPO_ROOT / "sdk" / "api-paths.json"

VERBS = ("get", "post", "put", "delete", "patch", "head", "options")


def read_annotations(lines: list[str]) -> list[dict[str, str]]:
    """Every `#[utoipa::path(...)]` block, paired with the fn it decorates."""
    operations: list[dict[str, str]] = []
    index = 0
    while index < len(lines):
        if lines[index].strip() != "#[utoipa::path(":
            index += 1
            continue

        depth = 0
        end = index
        while end < len(lines):
            depth += lines[end].count("(") - lines[end].count(")")
            if depth == 0 and end > index:
                break
            end += 1
        block = lines[index : end + 1]

        method = next(
            (line.strip().rstrip(",") for line in block if line.strip().rstrip(",") in VERBS),
            None,
        )
        path_match = next(
            (re.search(r'path\s*=\s*"([^"]+)"', line) for line in block if "path = " in line),
            None,
        )
        name_match = None
        for line in lines[end + 1 : end + 4]:
            name_match = re.search(r"\bfn\s+(\w+)\s*\(", line)
            if name_match:
                break

        if method is None or path_match is None or name_match is None:
            raise SystemExit(
                f"{SOURCE}:{index + 1}: could not read a verb, a path and a fn name "
                "out of this #[utoipa::path] block"
            )

        operations.append(
            {
                "operation_id": name_match.group(1),
                "method": method.upper(),
                "path": path_match.group(1),
            }
        )
        index = end + 1

    return operations


def read_apidoc_paths(text: str) -> list[str]:
    """The operation ids listed in `#[openapi(paths(...))]`."""
    start = text.index("#[openapi(")
    listed = text.index("paths(", start) + len("paths(")
    depth = 1
    end = listed
    while depth:
        if text[end] == "(":
            depth += 1
        elif text[end] == ")":
            depth -= 1
            if depth == 0:
                break
        end += 1
    body = re.sub(r"//[^\n]*", "", text[listed:end])
    return [item.strip() for item in body.split(",") if item.strip()]


def main() -> int:
    text = SOURCE.read_text(encoding="utf-8")
    annotated = read_annotations(text.split("\n"))
    listed = read_apidoc_paths(text)

    annotated_ids = {operation["operation_id"] for operation in annotated}
    listed_ids = set(listed)

    unpublished = sorted(annotated_ids - listed_ids)
    unknown = sorted(listed_ids - annotated_ids)
    if unpublished or unknown:
        for name in unpublished:
            print(
                f"error: {name} carries a #[utoipa::path] but is missing from "
                "ApiDoc paths(...), so it is absent from the served spec",
                file=sys.stderr,
            )
        for name in unknown:
            print(
                f"error: ApiDoc paths(...) lists {name}, which has no "
                "#[utoipa::path] annotation in this file",
                file=sys.stderr,
            )
        return 1

    operations = sorted(annotated, key=lambda op: (op["path"], op["method"]))
    snapshot = {
        "source": "src-tauri/src/api_server.rs",
        "regenerate_with": "python3 sdk/tools/extract-api-paths.py",
        "description": (
            "Every operation the desktop app publishes in its /openapi.json. The "
            "SDK test suites assert this list and their own coverage tables match "
            "exactly, so an endpoint added to the app fails the SDK tests until it "
            "is either wrapped or deliberately listed as omitted."
        ),
        "operation_count": len(operations),
        "operations": operations,
    }
    SNAPSHOT.write_text(json.dumps(snapshot, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {SNAPSHOT.relative_to(REPO_ROOT)} with {len(operations)} operations")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
