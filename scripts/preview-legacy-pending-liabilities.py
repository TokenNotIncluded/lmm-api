#!/usr/bin/env python3
"""Audit frozen legacy payment liabilities without opening a target or a provider.

The latest ArchDmit export contains two unresolved payment families:
``top_ups`` and ``subscription_orders``.  This command verifies the archive,
export and identity bindings, extracts only rows whose source status is
``pending``, and feeds them through :mod:`legacy_pending_liabilities`.

It is deliberately a preview/fence command.  It performs no PostgreSQL
connection, provider request, balance calculation, ledger write, or liability
close.  The default output is a redacted summary; ``--preview-output`` writes
the hash-bound raw-fact preview to a caller-selected private path.
"""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
from typing import Iterable


ROOT = Path(__file__).resolve().parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))


def _load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


F = _load("legacy_financial_preview", ROOT / "migrate-legacy-financial.py")
L = _load("legacy_pending_liabilities_preview", ROOT / "legacy_pending_liabilities.py")
I = F.IDENTITY


class PreviewError(ValueError):
    """A stable, non-secret validation failure."""


def _require(condition: bool, code: str) -> None:
    if not condition:
        raise PreviewError(code)


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
    except OSError as error:
        raise PreviewError("input_file_unreadable") from error
    return digest.hexdigest()


def _no_duplicate_keys(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise PreviewError("duplicate_json_key")
        result[key] = value
    return result


def _json_bytes(path: Path) -> bytes:
    try:
        return path.read_bytes()
    except OSError as error:
        raise PreviewError("input_file_unreadable") from error


def _read_json(path: Path) -> dict:
    try:
        value = json.loads(_json_bytes(path), object_pairs_hook=_no_duplicate_keys,
                           parse_float=F.Decimal,
                           parse_constant=lambda _: (_ for _ in ()).throw(PreviewError("nonfinite_json")))
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise PreviewError("json_invalid") from error
    _require(type(value) is dict, "json_object_required")
    return value


def _read_export(path: Path, selected: set[str]) -> tuple[dict[str, list[dict]], dict[str, Counter], str]:
    """Read one frozen JSONL export while hashing its exact bytes."""
    rows = {name: [] for name in selected}
    statuses = {name: Counter() for name in selected}
    digest = hashlib.sha256()
    try:
        stream = path.open("rb")
    except OSError as error:
        raise PreviewError("input_file_unreadable") from error
    with stream:
        for line in stream:
            digest.update(line)
            try:
                value = json.loads(line, object_pairs_hook=_no_duplicate_keys,
                                   parse_float=F.Decimal,
                                   parse_constant=lambda _: (_ for _ in ()).throw(PreviewError("nonfinite_json")))
            except (json.JSONDecodeError, UnicodeDecodeError) as error:
                raise PreviewError("export_jsonl_invalid") from error
            if not isinstance(value, dict) or value.get("kind") != "row":
                continue
            table = value.get("table")
            if table not in selected:
                continue
            row = value.get("row")
            _require(type(row) is dict, "export_row_invalid")
            _require(type(row.get("id")) is int and row["id"] > 0, "export_row_id_invalid")
            _require(type(row.get("user_id")) is int and row["user_id"] > 0, "export_row_user_invalid")
            status = row.get("status")
            _require(type(status) is str, "export_row_status_invalid")
            rows[table].append(row)
            statuses[table][status] += 1
    return rows, statuses, digest.hexdigest()


def _identity_binding(identity_map_path: Path, identity_map_bytes_path: Path,
                      identity_manifest_path: Path, *, source_archive_sha256: str,
                      source_export_sha256: str) -> tuple[dict[int, int], str, dict]:
    manifest = _read_json(identity_manifest_path)
    _require(manifest.get("scope") == "identity-only" and manifest.get("format_version") == 2,
             "identity_manifest_scope_invalid")
    _require(manifest.get("source_archive_sha256") == source_archive_sha256,
             "identity_archive_binding_mismatch")
    # The identity export is a different projection from the full financial
    # export.  Bind both to the same archive, but do not compare their file
    # hashes as if they were the same projection.
    _require(type(manifest.get("source_export_sha256")) is str and
             I.HEX.fullmatch(manifest["source_export_sha256"]) is not None,
             "identity_export_binding_invalid")
    _require(_sha256_file(identity_map_path) == manifest.get("mapping_file_sha256"),
             "identity_mapping_file_hash_mismatch")
    _require(_sha256_file(identity_map_bytes_path) == manifest.get("source_target_id_map_sha256"),
             "identity_id_map_file_hash_mismatch")
    mapping = _read_json(identity_map_path)
    _require(mapping.get("mapping_sha256") == manifest.get("mapping_sha256"),
             "identity_mapping_hash_mismatch")
    records = mapping.get("records")
    _require(type(records) is list, "identity_mapping_records_invalid")
    identities: dict[int, int] = {}
    accounts: set[int] = set()
    for record in records:
        _require(type(record) is dict, "identity_mapping_record_invalid")
        user_id, account_id = record.get("user_id"), record.get("account_id")
        _require(type(user_id) is int and user_id > 0, "identity_mapping_user_invalid")
        _require(type(account_id) is int and account_id > 0, "identity_mapping_account_invalid")
        _require(user_id not in identities and account_id not in accounts,
                 "identity_mapping_not_one_to_one")
        identities[user_id] = account_id
        accounts.add(account_id)
    _require(len(identities) > 0, "identity_mapping_empty")
    return identities, manifest["mapping_sha256"], manifest


def _check_financial_manifest(path: Path, *, archive_sha256: str, export_sha256: str,
                              identity_manifest: dict, identity_manifest_sha256: str,
                              identity_map_bytes_sha256: str) -> tuple[str, int, dict]:
    manifest = _read_json(path)
    _require(manifest.get("scope") == "financial-typed-planner-only",
             "financial_manifest_scope_invalid")
    _require(manifest.get("source_archive_sha256") == archive_sha256,
             "financial_archive_binding_mismatch")
    _require(manifest.get("source_export_sha256") == export_sha256,
             "financial_export_binding_mismatch")
    _require(manifest.get("identity_mapping_sha256") == identity_manifest.get("mapping_sha256"),
             "financial_identity_mapping_mismatch")
    _require(manifest.get("identity_manifest_file_sha256") == identity_manifest_sha256,
             "financial_identity_manifest_binding_mismatch")
    _require(manifest.get("identity_map_file_sha256") == identity_map_bytes_sha256,
             "financial_identity_map_binding_mismatch")
    observation = manifest.get("source_observation")
    _require(type(observation) is dict and type(observation.get("observed_at")) is int,
             "financial_observation_missing")
    _require(observation.get("exact_financial_snapshot_instant_known") is False and
             observation.get("must_not_be_used_as_original_payment_or_settlement_time") is True,
             "financial_observation_policy_invalid")
    return manifest["migration_id"], observation["observed_at"], manifest


def preview(*, source_export: Path, archive: Path, identity_map: Path,
            identity_manifest: Path, financial_manifest: Path,
            identity_map_bytes: Path | None = None,
            migration_id: str | None = None) -> tuple[dict, dict]:
    archive_sha256 = _sha256_file(archive)
    rows, statuses, export_sha256 = _read_export(source_export, {"top_ups", "subscription_orders"})
    identity_map_bytes = identity_map_bytes or identity_map.with_name("source-target-id-map.private.jsonl")
    identity_manifest_sha256 = _sha256_file(identity_manifest)
    identity_map_bytes_sha256 = _sha256_file(identity_map_bytes)
    identity, mapping_sha256, identity_doc = _identity_binding(
        identity_map, identity_map_bytes, identity_manifest,
        source_archive_sha256=archive_sha256, source_export_sha256=export_sha256,
    )
    default_migration_id, observed_at, financial_doc = _check_financial_manifest(
        financial_manifest, archive_sha256=archive_sha256, export_sha256=export_sha256,
        identity_manifest=identity_doc, identity_manifest_sha256=identity_manifest_sha256,
        identity_map_bytes_sha256=identity_map_bytes_sha256,
    )
    migration_id = migration_id or (default_migration_id + "-pending-liabilities")
    _require(type(migration_id) is str and I.MIGRATION_ID.fullmatch(migration_id) is not None,
             "pending_liability_migration_id_invalid")
    target_users = {user_id: user_id for user_id in identity}
    try:
        document = L.build(
            F, tables=rows, identities=identity, target_users=target_users,
            migration_id=migration_id, observed_at=observed_at,
            source_archive_sha256=archive_sha256, mapping_sha256=mapping_sha256,
            source_export_sha256=export_sha256,
        )
        L.verify(F, document)
    except (F.MappingError, ValueError, TypeError) as error:
        raise PreviewError(str(error)) from error
    summary = {
        "status": "PENDING_LIABILITIES_PRESERVED_SOURCE_ONLY",
        "gate": "BLOCKED_PENDING_PROVIDER_RECONCILIATION",
        "source_archive_sha256": archive_sha256,
        "source_export_sha256": export_sha256,
        "identity_mapping_sha256": mapping_sha256,
        "identity_source_export_sha256": identity_doc["source_export_sha256"],
        "migration_id": migration_id,
        "observed_at": observed_at,
        "source_table_counts": {table: len(values) for table, values in sorted(rows.items())},
        "source_status_counts": {table: dict(sorted(counter.items())) for table, counter in sorted(statuses.items())},
        "pending_record_count": len(document["records"]),
        "pending_counts_by_table": document["public_summary"]["counts_by_table"],
        "pending_counts_by_provider": document["public_summary"]["counts_by_provider"],
        "unmapped_owner_count": document["public_summary"]["unmapped_owner_count"],
        "provider_io_performed": False,
        "target_writes": 0,
        "ledger_writes": 0,
        "spendable_units_written": 0,
        "all_records_spendable": False,
        "all_records_settlement_state": "unknown",
        "reconciliation_required": True,
        "financial_manifest_sha256": _sha256_file(financial_manifest),
        "identity_manifest_sha256": _sha256_file(identity_manifest),
        "checks": [
            "archive_sha256_matches_manifest",
            "financial_export_sha256_matches_financial_manifest",
            "identity_projection_sha256_is_recorded_and_archive_bound",
            "identity_mapping_is_one_to_one",
            "all_pending_rows_are_hash_bound",
            "spendable_units_zero_and_ledger_writes_zero",
            "provider_io_not_performed",
        ],
        "fence_reasons": [
            "provider_status_not_observed",
            "provider_callbacks_and_external_payment_not_proven_terminal",
            "no_target_import_or_liability_close_authorized",
        ],
    }
    # Keep internal manifests referenced so future callers cannot accidentally
    # interpret this as a target or provider attestation.
    _ = financial_doc
    return summary, document


def main(argv: Iterable[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-export", type=Path, required=True)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--identity-map", type=Path, required=True)
    parser.add_argument("--identity-map-bytes", type=Path,
                        help="identity source-target JSONL; defaults beside --identity-map")
    parser.add_argument("--identity-manifest", type=Path, required=True)
    parser.add_argument("--financial-manifest", type=Path, required=True)
    parser.add_argument("--migration-id")
    parser.add_argument("--preview-output", type=Path,
                        help="write the private raw-fact preview to this path")
    args = parser.parse_args(argv)
    try:
        summary, document = preview(
            source_export=args.source_export, archive=args.archive,
            identity_map=args.identity_map, identity_manifest=args.identity_manifest,
            financial_manifest=args.financial_manifest, identity_map_bytes=args.identity_map_bytes,
            migration_id=args.migration_id,
        )
        if args.preview_output:
            args.preview_output.parent.mkdir(parents=True, exist_ok=True)
            args.preview_output.write_text(json.dumps(document, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n")
        print(json.dumps(summary, ensure_ascii=False, sort_keys=True, indent=2))
        return 0
    except PreviewError as error:
        print(json.dumps({"status": "REJECTED", "code": str(error), "target_writes": 0, "provider_io_performed": False}, sort_keys=True), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
