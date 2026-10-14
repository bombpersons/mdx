#!/usr/bin/env python3
"""Download and verify public Archive.org archives into a reusable test corpus."""
import argparse
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request


def hash_file(path, algorithm="sha256"):
    digest = hashlib.new(algorithm)
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def fetch_one(record, item, root, extractor):
    name = record["name"]
    if PurePosixPath(name).name != name or name in (".", ".."):
        raise ValueError(f"non-flat archive path: {name}")
    archive = root / "archives" / name
    expected_size = int(record["size"])
    expected_sha1 = record["sha1"]
    url = f"https://archive.org/download/{urllib.parse.quote(item)}/{urllib.parse.quote(name)}"
    valid = archive.is_file() and archive.stat().st_size == expected_size and hash_file(archive, "sha1") == expected_sha1
    if not valid:
        last_error = None
        for attempt in range(5):
            temporary = archive.with_suffix(".7z.part")
            try:
                request = urllib.request.Request(url, headers={"User-Agent": "mdx-test-library/1.0"})
                with urllib.request.urlopen(request, timeout=90) as response, temporary.open("wb") as output:
                    count = 0
                    while chunk := response.read(1024 * 1024):
                        count += len(chunk)
                        if count > expected_size:
                            raise ValueError("download exceeds declared archive size")
                        output.write(chunk)
                if temporary.stat().st_size != expected_size or hash_file(temporary, "sha1") != expected_sha1:
                    raise ValueError("download size/SHA-1 differs from original metadata")
                temporary.replace(archive)
                break
            except (OSError, ValueError, urllib.error.HTTPError) as error:
                last_error = error
                temporary.unlink(missing_ok=True)
                if attempt == 4:
                    raise RuntimeError(f"{name}: {last_error}") from error
                time.sleep(min(30, 2 ** attempt))
    destination = root / "roms" / archive.stem
    receipt = root / "receipts" / (record["sha1"] + ".json")
    if receipt.is_file():
        old = json.loads(receipt.read_text())
        if all(Path(entry["path"]).is_file() and hash_file(Path(entry["path"])) == entry["sha256"] for entry in old["roms"]):
            return old
    listing = subprocess.run([extractor, "l", "-slt", str(archive)], capture_output=True, text=True, check=True).stdout
    entries = listing.split("----------", 1)[1].strip().split("\n\n")
    for entry in entries:
        fields = dict(line.split(" = ", 1) for line in entry.splitlines() if " = " in line)
        relative = PurePosixPath(fields.get("Path", ""))
        if relative.is_absolute() or ".." in relative.parts or "Symbolic Link" in fields or "Hard Link" in fields:
            raise ValueError(f"unsafe archive member in {name}")
    with tempfile.TemporaryDirectory(prefix="extract-", dir=root) as temporary:
        subprocess.run([extractor, "x", "-y", f"-o{temporary}", str(archive)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        extracted = []
        for path in Path(temporary).rglob("*"):
            if path.is_symlink():
                raise ValueError(f"archive contains a symlink: {name}")
            if path.is_file():
                if path.suffix.lower() not in (".md", ".gen", ".bin", ".smd", ".32x"):
                    raise ValueError(f"unexpected non-ROM member: {name}/{path.name}")
                relative = path.relative_to(temporary)
                target = destination / relative
                extracted.append({"path": str(target), "member": str(relative), "bytes": path.stat().st_size, "sha256": hash_file(path)})
        if not extracted:
            raise ValueError(f"no ROM members in {name}")
        if destination.exists():
            if not all(Path(entry["path"]).is_file() and hash_file(Path(entry["path"])) == entry["sha256"] for entry in extracted):
                raise ValueError(f"existing extraction differs: {destination}")
        else:
            Path(temporary).rename(destination)
    result = {"archive": name, "url": url, "archive_bytes": expected_size, "archive_sha1": expected_sha1,
              "archive_sha256": hash_file(archive), "roms": extracted}
    receipt.write_text(json.dumps(result, indent=2) + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--item", default="nointro.md")
    parser.add_argument("--out", type=Path, default=Path("rom/library/nointro-md"))
    parser.add_argument("--jobs", type=int, default=6)
    args = parser.parse_args()
    root = args.out.resolve()
    for folder in (root, root / "archives", root / "roms", root / "receipts"):
        folder.mkdir(parents=True, exist_ok=True)
    metadata_path = root / "archive-metadata.json"
    if not metadata_path.exists():
        with urllib.request.urlopen(f"https://archive.org/metadata/{urllib.parse.quote(args.item)}", timeout=60) as response:
            metadata_path.write_bytes(response.read())
    metadata = json.loads(metadata_path.read_text())
    records = [record for record in metadata["files"] if record["name"].endswith(".7z") and not record.get("private")]
    extractor = shutil.which("7z") or shutil.which("7zz")
    if not extractor:
        raise SystemExit("7z or 7zz is required for verified extraction")
    results, errors = [], []
    print(json.dumps({"archives": len(records), "compressed_bytes": sum(int(r["size"]) for r in records), "output": str(root)}), flush=True)
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, min(args.jobs, 8))) as pool:
        futures = {pool.submit(fetch_one, record, args.item, root, extractor): record["name"] for record in records}
        for index, future in enumerate(concurrent.futures.as_completed(futures), 1):
            try:
                results.append(future.result())
            except Exception as error:
                errors.append({"archive": futures[future], "error": str(error)})
                print(json.dumps(errors[-1]), flush=True)
            if index % 100 == 0 or index == len(records):
                print(json.dumps({"processed": index, "verified": len(results), "failed": len(errors), "total": len(records)}), flush=True)
    manifest = {"schema": "mdx.test-rom-library.v1", "item": args.item,
                "source": f"https://archive.org/details/{args.item}", "metadata_sha256": hash_file(metadata_path),
                "expected_archives": len(records), "verified_archives": len(results), "complete": not errors,
                "archives": sorted(results, key=lambda result: result["archive"]), "errors": errors}
    (root / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"manifest": str(root / "manifest.json"), "complete": not errors, "verified": len(results), "errors": len(errors)}), flush=True)
    if errors:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
