#!/usr/bin/env python3
"""Find a version's notes in CHANGELOG.md, and build its GitHub release page from them."""

from pathlib import Path
import re
import sys

REPOSITORY = "https://github.com/sam-mfb/kashkeshet"
HEADING = re.compile(r"## (\d+\.\d+\.\d+)(?: - (.+))?")
RC = re.compile(r"-rc\.\d+$")
# Sections of dist's generated body that the page replaces: the changelog dist
# may have found itself, and its attestation steps, which docs/releases.md covers.
REPLACED = ("## Release Notes", "## Verifying GitHub Artifact Attestations")


def find(changelog, tag):
    """The name and notes of the section `## X.Y.Z[ - name]` for tag vX.Y.Z or vX.Y.Z-rc.N.

    The name is None when the heading gives none."""
    version = RC.sub("", tag.removeprefix("v"))
    found = None
    for number, line in enumerate(changelog.splitlines()):
        match = HEADING.fullmatch(line)
        if match and match.group(1) == version:
            if found is not None:
                raise ValueError(f"CHANGELOG.md has more than one section for {version}")
            found, name = number, match.group(2)
    if found is None:
        raise ValueError(f"CHANGELOG.md has no section for {version}; add its release notes under a line '## {version}' before tagging {tag}")
    lines = changelog.splitlines()[found + 1 :]
    end = next((i for i, line in enumerate(lines) if line.startswith("## ")), len(lines))
    notes = "\n".join(lines[:end]).strip()
    if not notes:
        raise ValueError(f"CHANGELOG.md's section for {version} is empty; write its release notes before tagging {tag}")
    return name, notes


def section(changelog, tag):
    """The notes under `## X.Y.Z` for tag vX.Y.Z or vX.Y.Z-rc.N, without the heading."""
    return find(changelog, tag)[1]


def named(changelog, tag):
    """The notes as the page shows them: a line `# <tag>: <name>` first when the
    section's heading gives a name, and the notes alone when it does not."""
    name, notes = find(changelog, tag)
    return notes if name is None else f"# {tag}: {name}\n\n{notes}"


def page(tag, notes, generated):
    """The release page: the notes, dist's install and download sections, and a link to the verification steps."""
    kept = []
    dropping = False
    for line in generated.splitlines():
        if line.startswith("## "):
            dropping = line.startswith(REPLACED)
        if not dropping:
            kept.append(line)
    body = "\n".join(kept).strip()
    verify = f"To verify a download, see [Verify a download]({REPOSITORY}/blob/{tag}/docs/releases.md#verify-a-download)."
    return f"{notes}\n\n{body}\n\n{verify}\n"


if __name__ == "__main__":
    changelog = Path(__file__).resolve().parents[2] / "CHANGELOG.md"
    try:
        if len(sys.argv) != 3:
            raise ValueError("usage: release-notes.py vX.Y.Z[-rc.N] GENERATED_BODY_FILE")
        tag, generated = sys.argv[1], Path(sys.argv[2])
        sys.stdout.write(page(tag, named(changelog.read_text(), tag), generated.read_text()))
    except (ValueError, OSError) as error:
        sys.exit(str(error))
