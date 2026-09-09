"""The SDK cannot silently drift from the app's API.

``sdk/api-paths.json`` is generated from ``src-tauri/src/api_server.rs`` and
lists every operation the desktop app publishes. These tests hold it against
the SDK's own table in both directions, so a new endpoint in the app fails here
until it is wrapped or deliberately omitted with a reason.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Dict, Set, Tuple

from donutbrowser import DonutClient
from donutbrowser.coverage import OMITTED, OPERATIONS

SNAPSHOT = Path(__file__).resolve().parents[2] / "api-paths.json"


def published() -> Set[Tuple[str, str]]:
    document: Dict[str, Any] = json.loads(SNAPSHOT.read_text(encoding="utf-8"))
    return {
        (operation["method"], operation["path"]) for operation in document["operations"]
    }


def test_the_snapshot_is_readable_and_not_empty() -> None:
    document = json.loads(SNAPSHOT.read_text(encoding="utf-8"))
    assert document["source"] == "src-tauri/src/api_server.rs"
    assert document["operation_count"] == len(document["operations"])
    assert document["operation_count"] > 0
    assert len(published()) == document["operation_count"], "the app has two identical operations"


def test_every_published_operation_is_wrapped_or_omitted() -> None:
    known = set(OPERATIONS) | set(OMITTED)
    missing = sorted(published() - known)
    assert not missing, (
        "the app publishes operations this SDK does not handle: "
        f"{missing}. Wrap each one, or add it to coverage.OMITTED with a reason."
    )


def test_the_sdk_claims_nothing_the_app_does_not_publish() -> None:
    stale = sorted((set(OPERATIONS) | set(OMITTED)) - published())
    assert not stale, (
        "this SDK handles operations the app no longer publishes: "
        f"{stale}. Regenerate the snapshot with sdk/tools/extract-api-paths.py, "
        "then drop or fix each entry."
    )


def test_an_operation_is_either_wrapped_or_omitted_but_not_both() -> None:
    both = sorted(set(OPERATIONS) & set(OMITTED))
    assert not both, f"listed twice: {both}"


def test_every_omission_gives_a_reason() -> None:
    for operation, reason in OMITTED.items():
        assert len(reason.strip()) > 40, f"{operation} is omitted without a real reason"


def test_every_wrapped_operation_names_a_real_method() -> None:
    for operation, method_name in OPERATIONS.items():
        attribute = getattr(DonutClient, method_name, None)
        assert callable(attribute), f"{operation} names {method_name}, which is not a method"


def test_no_two_operations_share_a_method() -> None:
    names = list(OPERATIONS.values())
    duplicates = sorted({name for name in names if names.count(name) > 1})
    assert not duplicates, f"one method is claimed by several operations: {duplicates}"
