#!/usr/bin/env python3
"""Build a deterministic, secret-free release-manifest.v1 from reviewed input."""

import argparse
import hashlib
import json
import re
from pathlib import Path

SHA = re.compile(r"[0-9a-f]{64}\Z")
COMMIT = re.compile(r"[0-9a-f]{40}\Z")
FIELDS = (
    "source_commit", "core_artifact_sha256", "extensions_artifact_sha256",
    "sbom_sha256", "schema_fingerprint_sha256", "provider_matrix_sha256",
    "identity_evidence_sha256", "financial_evidence_sha256", "edge_config_sha256",
    "runtime_release_identity_sha256", "runtime_source_commit",
    "runtime_core_artifact_sha256", "runtime_extensions_artifact_sha256",
    "previous_writer_epoch", "writer_epoch",
)


def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON field")
        result[key] = value
    return result


def parse(path, expected_source_commit=None):
    if (
        not path.is_file()
        or path.is_symlink()
        or path.stat().st_mode & 0o022
        or path.stat().st_size > 1024 * 1024
    ):
        raise ValueError("input must be a bounded regular file")
    value = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique)
    if isinstance(value, dict) and set(value) == {"release"}:
        value = value["release"]
    if not isinstance(value, dict) or set(value) != set(FIELDS):
        raise ValueError("release input fields are incomplete or unexpected")
    if not isinstance(value["source_commit"], str) or not COMMIT.fullmatch(value["source_commit"]):
        raise ValueError("source_commit must be a 40-character lowercase commit")
    if expected_source_commit is not None:
        if not isinstance(expected_source_commit, str) or not COMMIT.fullmatch(expected_source_commit):
            raise ValueError("expected_source_commit must be a 40-character lowercase commit")
        if value["source_commit"] != expected_source_commit:
            raise ValueError("source commit does not match expected candidate")
    if value["runtime_source_commit"] != value["source_commit"]:
        raise ValueError("runtime source commit mismatch")
    for field in (
        "core_artifact_sha256", "extensions_artifact_sha256", "sbom_sha256",
        "schema_fingerprint_sha256", "provider_matrix_sha256",
        "identity_evidence_sha256", "financial_evidence_sha256",
        "edge_config_sha256", "runtime_release_identity_sha256",
        "runtime_core_artifact_sha256", "runtime_extensions_artifact_sha256",
    ):
        if not isinstance(value[field], str) or not SHA.fullmatch(value[field]):
            raise ValueError(field + " must be a lowercase SHA-256")
    if value["runtime_core_artifact_sha256"] != value["core_artifact_sha256"]:
        raise ValueError("runtime Core artifact mismatch")
    if value["runtime_extensions_artifact_sha256"] != value["extensions_artifact_sha256"]:
        raise ValueError("runtime Extensions artifact mismatch")
    for field in ("previous_writer_epoch", "writer_epoch"):
        if type(value[field]) is not int or value[field] < 0:
            raise ValueError(field + " must be a non-negative integer")
    if value["writer_epoch"] <= value["previous_writer_epoch"]:
        raise ValueError("writer epoch must advance")
    return value


def build(input_path, output_path, expected_source_commit=None):
    if output_path.exists():
        raise ValueError("output already exists")
    value = {"protocol": "release-manifest.v1", **parse(input_path, expected_source_commit)}
    rendered = json.dumps(value, indent=2, sort_keys=True) + "\n"
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(rendered, encoding="utf-8")
    output_path.chmod(0o600)
    return hashlib.sha256(rendered.encode("utf-8")).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--expected-source-commit")
    args = parser.parse_args()
    try:
        print(build(args.input, args.output, args.expected_source_commit))
    except (OSError, TypeError, ValueError, json.JSONDecodeError) as error:
        parser.exit(2, "release manifest refused (" + type(error).__name__ + ")\n")


if __name__ == "__main__":
    raise SystemExit(main())
