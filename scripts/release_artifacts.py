"""Generate an SPDX SBOM, SLSA-style provenance statement and SHA256 manifest."""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import re
import subprocess
from pathlib import Path


def run(*command: str) -> str:
    return subprocess.check_output(command, text=True).strip()


def ref_id(value: str) -> str:
    return "SPDXRef-" + re.sub(r"[^A-Za-z0-9.-]", "-", value)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--tag", required=True)
    args = parser.parse_args()
    artifacts = sorted(path for path in args.artifacts.iterdir() if path.is_file())
    if not artifacts:
        parser.error("no release artifacts found")

    metadata = json.loads(run("cargo", "metadata", "--locked", "--format-version", "1"))
    root_package = next(
        package for package in metadata["packages"]
        if package["id"] == metadata["resolve"]["root"]
    )
    expected_tag = f"v{root_package['version']}"
    if args.tag != expected_tag:
        parser.error(f"tag {args.tag!r} does not match package version {root_package['version']!r}")
    if run("git", "status", "--porcelain", "--untracked-files=no"):
        parser.error("refusing to create release provenance from a dirty tracked worktree")
    head_commit = run("git", "rev-parse", "HEAD")
    try:
        tagged_commit = run("git", "rev-parse", f"{args.tag}^{{commit}}")
    except subprocess.CalledProcessError:
        parser.error(f"release tag {args.tag!r} does not exist locally")
    if tagged_commit != head_commit:
        parser.error(f"HEAD {head_commit} does not match tag {args.tag} at {tagged_commit}")
    packages = []
    package_refs = {}
    relationships = []
    for package in metadata["packages"]:
        name, version = package["name"], package["version"]
        package_ref = ref_id(f"{name}-{version}")
        package_refs[package["id"]] = package_ref
        checksums = []
        if package.get("checksum"):
            checksums.append({"algorithm": "SHA256", "checksumValue": package["checksum"]})
        item = {
            "name": name,
            "SPDXID": package_ref,
            "versionInfo": version,
            "downloadLocation": package.get("source") or "NOASSERTION",
            "filesAnalyzed": False,
            "licenseConcluded": "NOASSERTION",
            "licenseDeclared": package.get("license") or "NOASSERTION",
            "copyrightText": "NOASSERTION",
        }
        if checksums:
            item["checksums"] = checksums
        if package.get("description"):
            item["summary"] = package["description"][:1024]
        packages.append(item)
    root_id = package_refs[metadata["resolve"]["root"]]
    for node in metadata["resolve"]["nodes"]:
        source = package_refs[node["id"]]
        for dependency in node["deps"]:
            target = package_refs.get(dependency["pkg"])
            if target:
                relationships.append({"spdxElementId": source, "relationshipType": "DEPENDS_ON", "relatedSpdxElement": target})
    now = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    args.output.mkdir(parents=True, exist_ok=True)
    sbom = {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": f"serpentype {args.tag}",
        "documentNamespace": f"https://github.com/Otakunavi/serpentype/sbom/{args.tag}/{head_commit}",
        "creationInfo": {"creators": ["Tool: scripts/release_artifacts.py"], "created": now},
        "packages": packages,
        "relationships": [{"spdxElementId": "SPDXRef-DOCUMENT", "relationshipType": "DESCRIBES", "relatedSpdxElement": root_id}, *relationships],
    }
    (args.output / "sbom.spdx.json").write_text(json.dumps(sbom, indent=2, sort_keys=True) + "\n")

    subjects = [{"name": path.name, "digest": {"sha256": sha256(path)}} for path in artifacts]
    provenance = {
        "_type": "https://in-toto.io/Statement/v1",
        "subject": subjects,
        "predicateType": "https://slsa.dev/provenance/v1",
        "predicate": {
            "buildDefinition": {
                "buildType": "https://github.com/Otakunavi/serpentype/.github/workflows/publish-pypi.yml@v1",
                "externalParameters": {"releaseTag": args.tag},
                "internalParameters": {"repository": run("git", "config", "--get", "remote.origin.url")},
                "resolvedDependencies": [{"uri": "git+https://github.com/Otakunavi/serpentype", "digest": {"gitCommit": head_commit}}],
            },
            "runDetails": {
                "builder": {"id": "https://github.com/actions/runner"},
                "metadata": {"invocationId": "NOASSERTION", "startedOn": now, "finishedOn": now},
                "byproducts": [{"name": "rustc -Vv", "value": run("rustc", "-Vv")}],
            },
        },
    }
    (args.output / "provenance.intoto.jsonl").write_text(json.dumps(provenance, sort_keys=True) + "\n")
    lines = [f"{sha256(path)}  {path.name}" for path in artifacts]
    for path in sorted(args.output.iterdir()):
        if path.is_file() and path.name != "SHA256SUMS":
            lines.append(f"{sha256(path)}  {path.name}")
    (args.output / "SHA256SUMS").write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
