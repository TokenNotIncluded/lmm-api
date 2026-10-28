#!/usr/bin/env python3
"""Build sealed synthetic payment inputs and invoke the owning test runner."""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output = args.output
    if not output.is_absolute() or output.exists() or output.parent.resolve() != output.parent:
        parser.error("output must be a new absolute path with resolved parents")
    output.mkdir(mode=0o700)
    spec = importlib.util.spec_from_file_location("live_payment", ROOT / "tests/payments/verify_live_process.py")
    live = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(live)
    frozen = live.v.sources()
    inputs = output / "compiled-inputs.json"
    live.v.write_private(inputs, json.dumps(frozen, sort_keys=True))
    env = live.v.clean_environment()
    # Freeze before building all four inputs; do not adopt ambient binaries.
    build = subprocess.run(
        ["cargo", "test", "--locked", "--test", "payments_live_process", "--no-run", "--message-format=json"],
        cwd=ROOT / "services/core", env=env, check=True, stdout=subprocess.PIPE, text=True,
    )
    artifacts = [json.loads(line) for line in build.stdout.splitlines()]
    binaries = {}
    for kind, name, test in (("core", "lmm-core", False), ("test", "payments_live_process", True)):
        matches = [item["executable"] for item in artifacts
                   if item.get("reason") == "compiler-artifact" and item.get("executable")
                   and item["target"]["name"] == name and item["profile"]["test"] == test]
        if len(matches) != 1:
            raise RuntimeError("missing unique compiled artifact: " + kind)
        binaries[kind] = Path(matches[0]).resolve(strict=True)
    env["CGO_ENABLED"] = "0"
    for kind, command in (("go", ["go", "build", "-mod=readonly", "-trimpath"]),
                          ("bootstrap", ["go", "test", "-mod=readonly", "-c"])):
        binaries[kind] = output / ("payment-" + kind)
        package = "./cmd/extensions" if kind == "go" else "./internal/modules/payments"
        subprocess.run([*command, "-o", str(binaries[kind]), package], cwd=ROOT / "services/extensions", env=env, check=True)
    if live.v.sources() != frozen:
        raise RuntimeError("source drift during compilation")
    command = [sys.executable, "-B", str(ROOT / "tests/payments/verify_live_process.py"),
               "--output", str(output / "evidence"), "--compiled-inputs", str(inputs)]
    for kind, binary in binaries.items():
        command.extend(["--" + kind + "-binary", str(binary), "--" + kind + "-sha256", live.sha(binary)])
    # This Python runner, not cargo or a fabricated marker, must own the test.
    return subprocess.run(command, cwd=ROOT, env=env).returncode


if __name__ == "__main__":
    sys.exit(main())
