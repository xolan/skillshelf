#!/usr/bin/env python3
"""Writes THIRD_PARTY_LICENSES.md: the assets and Rust crates skillshelf ships, with their licenses.

Run it through `mise run licenses`. With `--check`, fails instead of writing when the file is out
of date. Uses only `cargo` and the crate sources Cargo has already downloaded.
"""

import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "THIRD_PARTY_LICENSES.md"
LICENSE_PREFIXES = ("LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE")
# Platforms release builds ship for; add a triple here when a new one ships.
TARGETS = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"]

# Files the app embeds that are not Rust crates: (name, license, source, license file).
ASSETS = [
    (
        "Geist and Geist Mono fonts",
        "OFL-1.1",
        "https://github.com/vercel/geist-font",
        "crates/skillshelf/fonts/OFL.txt",
    ),
    (
        "Base16 color schemes",
        "MIT",
        "https://github.com/tinted-theming/schemes",
        "crates/skillshelf/themes/base16/LICENSE",
    ),
    ("Lucide icons, via gpui-kit-assets", "ISC", "https://lucide.dev", None),
]


def cargo(*args):
    return subprocess.run(
        ["cargo", *args], cwd=ROOT, check=True, capture_output=True, text=True
    ).stdout


def shipped_packages(metadata):
    """Packages reachable from the workspace through normal and build dependencies on one platform."""
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    seen = set()
    stack = list(metadata["workspace_members"])
    while stack:
        node = nodes[stack.pop()]
        for dep in node["deps"]:
            kinds = {kind["kind"] for kind in dep["dep_kinds"]}
            if kinds != {"dev"} and dep["pkg"] not in seen:
                seen.add(dep["pkg"])
                stack.append(dep["pkg"])
    members = set(metadata["workspace_members"])
    packages = [p for p in metadata["packages"] if p["id"] in seen and p["id"] not in members]
    return sorted(packages, key=lambda p: (p["name"].lower(), p["version"]))


def license_texts(package):
    root = Path(package["manifest_path"]).parent
    files = {f for f in root.iterdir() if f.is_file() and f.name.upper().startswith(LICENSE_PREFIXES)}
    if package.get("license_file"):
        extra = root / package["license_file"]
        if extra.is_file():
            files.add(extra)
    return [normalize(f.read_text(errors="replace")) for f in sorted(files)]


# "Copyright (c) 2020 …", "Copyright 2020 …", "© 2020 …"; not prose that starts with "copyright".
COPYRIGHT_LINE = re.compile(r"\s*(copyright\s*(\(c\)|©|\d{4})|\(c\)\s*\d{4}|©\s*\d{4})", re.IGNORECASE)


def split_copyright(text):
    """The copyright lines of a license text, and the rest of it."""
    lines = text.split("\n")
    is_copyright = lambda line: COPYRIGHT_LINE.match(line) is not None
    notices = [line.strip() for line in lines if is_copyright(line)]
    body = normalize("\n".join(line for line in lines if not is_copyright(line)))
    # The Apache appendix only explains how to apply the license; variants differ there.
    end = body.find("END OF TERMS AND CONDITIONS")
    if end != -1:
        body = body[: end + len("END OF TERMS AND CONDITIONS")]
    # A leading title line ("MIT License", "The MIT License (MIT)") is shown as the heading.
    rest = body.split("\n")
    while rest and len(rest[0].split()) <= 6 and "license" in rest[0].lower():
        rest = rest[1:]
    return notices, normalize("\n".join(rest))


# Phrases that identify a license text, checked in order.
KNOWN_LICENSES = [
    ("Apache License", "Apache License 2.0"),
    ("Mozilla Public License", "Mozilla Public License 2.0"),
    ("Boost Software License", "Boost Software License 1.0"),
    ("UNICODE LICENSE", "Unicode License v3"),
    ("Zero Clause", "BSD Zero Clause License"),
    ("free and unencumbered software", "The Unlicense"),
    ("Permission is hereby granted, free of charge", "MIT License"),
    ("Permission to use, copy, modify, and/or distribute", "ISC License"),
    ("provided 'as-is'", "zlib License"),
    ("Neither the name", "BSD 3-Clause License"),
    ("Redistribution and use in source and binary forms", "BSD 2-Clause License"),
]


def license_name(body):
    flat = " ".join(body.split())
    for phrase, name in KNOWN_LICENSES:
        if phrase.lower() in flat.lower():
            return name
    return "Other license and notices"


def normalize(text):
    lines = [line.rstrip() for line in text.replace("\r\n", "\n").split("\n")]
    return "\n".join(lines).strip("\n")


def fence(text):
    # A fence longer than any backtick run inside the text.
    longest = max((len(run) for run in text.split("\n") if set(run) == {"`"}), default=0)
    ticks = "`" * max(3, longest + 1)
    return f"{ticks}text\n{text}\n{ticks}"


def render():
    cargo("fetch", "--locked")
    by_id = {}
    for target in TARGETS:
        metadata = json.loads(
            cargo("metadata", "--format-version", "1", "--locked", "--filter-platform", target)
        )
        by_id.update({p["id"]: p for p in shipped_packages(metadata)})
    packages = sorted(by_id.values(), key=lambda p: (p["name"].lower(), p["version"]))

    out = [
        "# Third-party licenses",
        "",
        "skillshelf is released under the [MIT License](LICENSE). It is built on the work below;",
        "thank you to everyone who made it.",
        "",
        "This file is generated by `mise run licenses`; do not edit it by hand.",
        "",
        "## Bundled assets",
        "",
        "| Asset | License | Source |",
        "| --- | --- | --- |",
    ]
    for name, spdx, url, _ in ASSETS:
        out.append(f"| {name} | {spdx} | <{url}> |")

    out += [
        "",
        "## Rust crates",
        "",
        f"{len(packages)} crates, built into the {' and '.join(TARGETS)} releases.",
        "",
    ]
    out += ["| Crate | Version | License |", "| --- | --- | --- |"]
    # Texts that differ only in their copyright lines share one entry listing those lines.
    groups = {}  # whitespace-insensitive body -> (name, body, crates, copyright lines)
    missing = []
    for package in packages:
        name = package["name"]
        link = package.get("repository") or f"https://crates.io/crates/{name}"
        out.append(f"| [{name}]({link}) | {package['version']} | {package.get('license') or 'see repository'} |")
        texts = license_texts(package)
        if not texts:
            missing.append(package)
        for text in texts:
            notices, body = split_copyright(text)
            key = re.sub(r"https?://", "", " ".join(body.split()))
            _, _, users, lines = groups.setdefault(key, (license_name(text), body, [], []))
            users.append(f"{name} {package['version']}")
            lines += [n for n in notices if n not in lines]

    out += ["", "## License texts", ""]
    for name, spdx, _, path in ASSETS:
        if path:
            out += [f"### {name} ({spdx})", "", fence(normalize((ROOT / path).read_text())), ""]
    for name, body, users, notices in groups.values():
        out += [f"### {name}", "", "Used by " + ", ".join(users) + ".", ""]
        if notices:
            out += ["Copyright notices:", ""] + [f"- {n}" for n in notices] + [""]
        out += [fence(body), ""]
    if missing:
        out += [
            "### Crates without a license file",
            "",
            "These crates publish no license file; their license is the one listed above, and the",
            "full text is in their repository.",
            "",
        ]
        out += [f"- {p['name']} {p['version']} ({p.get('license') or 'unknown'})" for p in missing]
        out.append("")
    return "\n".join(out).rstrip("\n") + "\n"


def selftest():
    notices, body = split_copyright(
        "MIT License\n\nCopyright (c) 2020 A\n© 2021 B\nThe above copyright notice shall be included."
    )
    assert notices == ["Copyright (c) 2020 A", "© 2021 B"], notices
    assert body == "The above copyright notice shall be included.", body
    assert license_name("Permission is hereby granted, free of charge, to any") == "MIT License"
    assert fence("a\n```\nb").startswith("````text")


def main():
    selftest()
    text = render()
    if "--check" in sys.argv[1:]:
        if not OUTPUT.exists() or OUTPUT.read_text() != text:
            sys.exit("THIRD_PARTY_LICENSES.md is out of date; run `mise run licenses`.")
        return
    OUTPUT.write_text(text)


if __name__ == "__main__":
    main()
