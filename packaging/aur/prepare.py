#!/usr/bin/env python3
"""Generate the two AUR binary recipes from verified component archives.

Runs only on the build machine. Does not publish, install, execute the input
binary, start services, or change a database. makepkg generates .SRCINFO.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
from urllib.parse import quote

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("native_package", ROOT / "packaging/linux/build.py")
NATIVE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(NATIVE)
PACKAGE_NAMES = {"core": "lmm-api-core-bin", "extensions": "lmm-api-extensions-bin"}


def shell(value: str) -> str:
    return "'" + value.replace("'", "'\\''") + "'"


def prepare(args: argparse.Namespace, root: Path = ROOT) -> Path:
    component, program = args.component, f"lmm-{args.component}"
    name = PACKAGE_NAMES[component]
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._/-]*", args.tag):
        raise ValueError("invalid GitHub release tag")
    if len(args.archive) != len(args.sha256) or not args.archive:
        raise ValueError("each archive needs exactly one trusted --sha256, in the same order")
    if not 1 <= args.release <= 999999:
        raise ValueError("invalid packaging release")
    destination = args.output.resolve()
    if destination.exists() or destination.is_symlink():
        raise FileExistsError("AUR output exists; generate into a new directory")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".aur-prepare-", dir=destination.parent) as work:
        work = Path(work)
        result = work / "recipe"
        result.mkdir()
        archives, versions, revisions = {}, set(), set()
        for index, (archive, expected) in enumerate(zip(args.archive, args.sha256, strict=True)):
            _, info = NATIVE.prepare(root, archive.resolve(), expected, args.revision,
                                     component, "archlinux", args.release, work / f"stage-{index}")
            expected_name = f"{program}-{info['version']}-linux-{info['arch']}.tar.gz"
            if archive.name != expected_name:
                raise ValueError("archive filename differs from the upstream distribution contract")
            arch = {"amd64": "x86_64", "arm64": "aarch64"}[info["arch"]]
            if arch in archives:
                raise ValueError("duplicate architecture")
            archives[arch] = {"archive": archive.name, "sha256": expected, "info": info}
            versions.add(info["version"])
            revisions.add(info["source_revision"])
        if len(versions) != 1 or revisions != {args.revision}:
            raise ValueError("archives must describe the same version and source revision")
        version = versions.pop()
        pkgver, pkgrel = NATIVE.native_version(version, args.release, "archlinux")
        assets = {
            f"{program}.service": NATIVE.template(root, "service.systemd", {
                "PACKAGE": program, "COMPONENT": component,
                "MEMORY": "256M" if component == "core" else "192M"}),
            f"{program}.sysusers": NATIVE.template(root, "service.sysusers", {
                "PACKAGE": program, "COMPONENT": component}),
            "rpc.service.conf.example": NATIVE.template(root, f"rpc-{component}.systemd", {}),
        }
        if component == "core":
            assets[f"{program}.sysusers"] += b"g lmm-rpc -\n"
        instructions = (
            f"# {program} installation\n\n"
            f"Configure /etc/lmm.conf.d/{component}.toml before starting {program}.service.\n"
            "Private credential files must be provisioned separately for the matching service user.\n"
            "See filesystem-layout.md and the upstream archive README for ownership and initialization.\n"
            "The package does not initialize/migrate databases, enable services, or restart services.\n"
            "Arch's normal sysusers hook may provision the declared non-login service account.\n"
            "Configuration upgrades use pacman's backup handling; review any .pacnew file.\n"
            "Removing the package does not remove private credentials, state, or database records.\n"
            "Stop the intended service explicitly before final removal.\n"
            "For shared RPC, review rpc.service.conf.example and provision separate purpose tokens.\n"
            "The Core works independently; extensions are optional.\n"
        )
        assets["INSTALL.md"] = instructions.encode()
        buildinfo = {"schema_version": 1, "package": name, "program": program,
                     "version": version, "pkgver": pkgver, "pkgrel": pkgrel,
                     "source_revision": args.revision, "release_tag": args.tag,
                     "architectures": {a: {k: value[k] for k in ("archive", "sha256")}
                                       for a, value in sorted(archives.items())}}
        assets["BUILDINFO.json"] = (json.dumps(buildinfo, indent=2, sort_keys=True) + "\n").encode()
        for filename, data in assets.items():
            (result / filename).write_bytes(data)
        lines = [
            "# Maintainer: LIghtJUNction <support@lmm.best>",
            "# SPDX-License-Identifier: AGPL-3.0-or-later",
            "# Generated by packaging/aur/prepare.py from verified upstream archives.",
            "# shellcheck shell=bash disable=SC2034,SC2154",
            f"pkgname={name}", f"pkgver={pkgver}", f"pkgrel={pkgrel}",
            f"pkgdesc={shell('LMM API ' + ('Rust core' if component == 'core' else 'Go extensions') + ' (prebuilt binary)')}",
            "arch=(" + " ".join(shell(a) for a in sorted(archives)) + ")",
            "url='https://github.com/TokenNotIncluded/api.lmm.best'",
            "license=('AGPL-3.0-or-later')",
            f'provides=("lmm-api-{component}=${{pkgver}}" "{program}=${{pkgver}}")',
            "conflicts=(" + " ".join(shell(p) for p in (program, f"{program}-bin", f"lmm-api-{component}")) + ")",
            f"backup=('etc/lmm.conf.d/{component}.toml')",
            "options=('!strip' '!debug')",
            "source=(" + " ".join(shell(f) for f in assets) + ")",
            "sha256sums=(" + " ".join(shell(hashlib.sha256(b).hexdigest()) for b in assets.values()) + ")",
        ]
        for arch, data in sorted(archives.items()):
            dependencies = data["info"]["dependencies"]
            lines += ["depends_" + arch + "=(" + " ".join(shell(d) for d in dependencies) + ")",
                      f"source_{arch}=({shell('https://github.com/TokenNotIncluded/api.lmm.best/releases/download/' + quote(args.tag, safe='') + '/' + data['archive'])})",
                      f"sha256sums_{arch}=({shell(data['sha256'])})"]
        lines += ["", "package() {", '  cd "$srcdir" || return 1']

        def install(source: str, target: str, mode: str = "644") -> None:
            lines.append(f'  install -Dm{mode} {shell(source)} "$pkgdir/{target}"')

        install(f"bin/{program}", f"usr/bin/{program}", "755")
        install(f"config/{component}.example.toml", f"etc/lmm.conf.d/{component}.toml")
        for path in ("LICENSE", "NOTICE", "THIRD-PARTY-LICENSES.md"):
            install(path, f"usr/share/licenses/{program}/{path}")
        install("README.md", f"usr/share/doc/{program}/README.archive.md")
        install("docs/filesystem-layout.md", f"usr/share/doc/{program}/filesystem-layout.md")
        for filename in ("BUILDINFO.json", "INSTALL.md", "rpc.service.conf.example"):
            install(filename, f"usr/share/doc/{program}/{filename}")
        install(f"{program}.service", f"usr/lib/systemd/system/{program}.service")
        install(f"{program}.sysusers", f"usr/lib/sysusers.d/{program}.conf")
        if component == "extensions":
            resources = json.loads((root / "packaging/distribution.json").read_text())["components"][component]["resources"]
            for path in resources:
                if path.startswith("config/legal/"):
                    install(path, "usr/share/lmm/legal/" + path.removeprefix("config/legal/"))
                elif path == "docs/legal/README.md":
                    install(path, f"usr/share/{program}/{path}")
        lines += ["}", ""]
        (result / "PKGBUILD").write_text("\n".join(lines))
        subprocess.run(["bash", "-n", "PKGBUILD"], cwd=result, check=True)
        metadata = subprocess.run(["makepkg", "--printsrcinfo"], cwd=result, check=True,
                                  stdout=subprocess.PIPE, text=True).stdout
        (result / ".SRCINFO").write_text(metadata)
        os.rename(result, destination)
    return destination


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--component", required=True, choices=PACKAGE_NAMES)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--archive", required=True, action="append", type=Path)
    parser.add_argument("--sha256", required=True, action="append")
    parser.add_argument("--release", type=int, default=1)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        print(prepare(args))
    except (ValueError, FileExistsError, subprocess.CalledProcessError) as error:
        parser.exit(2, f"aur recipe: {error}\n")


if __name__ == "__main__":
    main()
