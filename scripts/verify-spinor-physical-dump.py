#!/usr/bin/env python3
"""Compare an independently acquired physical-zero NOR dump, without hardware I/O.

The caller must retain the physical reader command, identity and acquisition log.
A byte match here does NOT authenticate the dump's provenance or verify boot.
Never feed this tool a dump obtained through the same unproven FES Flash mapping.
"""

import argparse
import hashlib
import json
from pathlib import Path


def digest(data):
    return hashlib.sha256(data).hexdigest()


def compare(image, physical_dump, application_offset=0x80000):
    if not 0 < application_offset < len(image):
        raise ValueError("application offset must split the complete image")
    if len(physical_dump) < len(image):
        raise ValueError("physical-zero dump does not cover the complete target range")
    regions = []
    for name, start, end in (
        ("boot", 0, application_offset),
        ("application", application_offset, len(image)),
        ("complete_image", 0, len(image)),
    ):
        expected, actual = image[start:end], physical_dump[start:end]
        regions.append({
            "name": name, "physicalStart": start, "byteLength": end - start,
            "expectedSha256": digest(expected), "actualSha256": digest(actual),
            "matches": actual == expected,
        })
    prefix = image[:min(1024, len(image))]
    first_header = physical_dump.find(prefix)
    return {
        "byteComparison": "pass" if all(r["matches"] for r in regions) else "fail",
        "physicalReaderProvenance": "requires_separate_acquisition_evidence",
        "bootVerification": "not_verified",
        "imageBytes": len(image), "dumpBytes": len(physical_dump),
        "imageSha256": digest(image), "dumpSha256": digest(physical_dump),
        "firstImageHeaderOffsetInDump": first_header if first_header >= 0 else None,
        "regions": regions,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", type=Path, required=True)
    parser.add_argument("--physical-zero-dump", type=Path, required=True)
    parser.add_argument("--application-offset", type=lambda n: int(n, 0), default=0x80000)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    # Evidence reports must not overwrite any pre-existing artifact or input.
    if args.report.exists():
        parser.error("report already exists; choose a new evidence filename")
    try:
        result = compare(args.image.read_bytes(), args.physical_zero_dump.read_bytes(), args.application_offset)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    with args.report.open("x", encoding="utf-8") as output:
        json.dump(result, output, ensure_ascii=False, indent=2)
        output.write("\n")
    print(json.dumps(result, ensure_ascii=False))
    return 0 if result["byteComparison"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
