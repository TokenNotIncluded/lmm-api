#!/usr/bin/env python3
"""Validate and package a recipe release bound to a published implementation."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib

SOURCE_REPOSITORY = "TokenNotIncluded/lmm-api-source"
RECIPKG_REVISION = "7b7f11ce4aaa7d63f922c57a82b227c8aa6b1f3f"
REQUEST_FIELDS = {"version", "core", "source_version", "source_revision"}


def command(*args, cwd=None):
    return subprocess.check_output(args, cwd=cwd, text=True).strip()


def package_version(root, name):
    text = (root / "recipes" / name / "RECIPE.md").read_text()
    blocks = re.findall(r"^```toml\n(.*?)^```", text, re.MULTILINE | re.DOTALL)
    if len(blocks) != 1:
        raise ValueError(f"{name}: expected one TOML manifest")
    package = tomllib.loads(blocks[0])["package"]
    if package["name"] != name:
        raise ValueError(f"{name}: package identity differs")
    return package["version"]


def validate_request(root, request, source_revision):
    if not isinstance(request, dict) or set(request) != REQUEST_FIELDS:
        raise ValueError("release request must contain exactly version, core, source_version and source_revision")
    for key in ("version", "core", "source_version"):
        if not isinstance(request[key], str) or not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", request[key]):
            raise ValueError(f"{key}: expected a numeric release version")
    if not isinstance(request["source_revision"], str) or not re.fullmatch(r"[0-9a-f]{40}", request["source_revision"]):
        raise ValueError("source_revision: expected a full commit SHA")
    if request["source_revision"] != source_revision:
        raise ValueError("release request and implementation gitlink differ")
    for name, field in (("lmm-api", "version"), ("lmm-core", "core")):
        if package_version(root, name) != request[field]:
            raise ValueError(f"{name}: request and recipe versions differ")
    return request


def github(repository, path):
    return json.loads(command("gh", "api", f"repos/{repository}/{path}"))


def published_source(request):
    tag = "v" + request["source_version"]
    release = github(SOURCE_REPOSITORY, "releases/tags/" + tag)
    if release["draft"] or not release.get("published_at") or release["tag_name"] != tag:
        raise ValueError("implementation release is not published")
    target = github(SOURCE_REPOSITORY, "git/ref/tags/" + tag)["object"]
    for _ in range(8):
        if target["type"] != "tag":
            break
        target = github(SOURCE_REPOSITORY, "git/tags/" + target["sha"])["object"]
    if target["type"] != "commit" or target["sha"] != request["source_revision"]:
        raise ValueError("published implementation tag does not match the gitlink")
    return release["html_url"]


def prepare(root, cli, output):
    request = json.loads((root / ".github/recipe-release-request.json").read_text())
    entry = command("git", "ls-tree", "HEAD", "generated/lmm-api-source", cwd=root).split()
    if len(entry) != 4 or entry[:2] != ["160000", "commit"]:
        raise ValueError("implementation must be a pinned gitlink")
    validate_request(root, request, entry[2])
    source_url = command("git", "config", "-f", ".gitmodules", "--get", "submodule.generated/lmm-api-source.url", cwd=root)
    if source_url != "https://github.com/" + SOURCE_REPOSITORY + ".git":
        raise ValueError("implementation submodule repository differs")
    revision = command("git", "rev-parse", "HEAD", cwd=root)
    if revision != os.environ["GITHUB_SHA"]:
        raise ValueError("checkout differs from the workflow revision")
    subprocess.run(["git", "diff", "--exit-code", "HEAD"], cwd=root, check=True)
    if command("git", "rev-parse", "HEAD", cwd=cli) != RECIPKG_REVISION:
        raise ValueError("Recipkg checkout differs from the reviewed tool revision")
    release_url = published_source(request)
    output.mkdir(parents=True, exist_ok=False)
    environment = {**os.environ, "PYTHONPATH": str(cli / "src")}
    log = []
    for name, field in (("lmm-api", "version"), ("lmm-core", "core")):
        archive = output / f"{name}-{request[field]}.recipe.tar.gz"
        for args in (("check", str(root / "recipes" / name)), ("pack", str(root / "recipes" / name), "--output", str(archive)), ("check", str(archive))):
            result = subprocess.run([sys.executable, "-B", "-m", "recipkg", *args], env=environment, capture_output=True, text=True, check=True)
            log.extend(("recipkg " + " ".join(args), result.stdout, result.stderr))
    prompt = subprocess.run([sys.executable, "-B", "-m", "recipkg", "build", str(output / f'lmm-api-{request["version"]}.recipe.tar.gz'), "--lang", "zh-CN"], env=environment, capture_output=True, text=True, check=True)
    if not prompt.stdout.strip():
        raise ValueError("recursive build prompt is empty")
    log.append("Recursive build prompt generated successfully; no application was compiled or executed.\n")
    (output / "recipe-checks.log").write_text("\n".join(log))
    subprocess.run(["git", "archive", "--format=tar.gz", f'--prefix=lmm-api-recipes-{request["version"]}/', f'--output={output / ("lmm-api-recipes-" + request["version"] + ".tar.gz")}', "HEAD"], cwd=root, check=True)
    record = {**request, "repository": os.environ["GITHUB_REPOSITORY"], "revision": revision, "source_repository": SOURCE_REPOSITORY, "source_release": release_url, "recipkg_revision": RECIPKG_REVISION, "artifact_kind": "Markdown recipes and parent repository source", "qualification": "Recipe structure, recursive archives and prompt generation; no application acceptance or deployment", "workflow_run": f'https://github.com/{os.environ["GITHUB_REPOSITORY"]}/actions/runs/{os.environ["GITHUB_RUN_ID"]}'}
    (output / "source.json").write_text(json.dumps(record, indent=2) + "\n")
    checksums = [f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n" for path in sorted(output.iterdir())]
    (output / "SHA256SUMS").write_text("".join(checksums))
    with open(os.environ["GITHUB_OUTPUT"], "a") as handle:
        handle.write(f'version={request["version"]}\nrevision={revision}\n')
    print(f'Qualified recipes-v{request["version"]} at {revision}, implementation {request["source_revision"]}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    prepare(Path(__file__).resolve().parents[1], args.cli.resolve(), args.output.resolve())


if __name__ == "__main__":
    main()
