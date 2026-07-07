#!/usr/bin/env python3
"""Validate an OKF v0.1 bundle (the subset of the spec this repo's bundles use).

Checks:
1. every non-reserved .md file has parseable YAML frontmatter with a non-empty `type`
2. index.md files carry no frontmatter, except the bundle-root index.md, which may
   declare only `okf_version`; log.md carries no frontmatter
3. all intra-bundle markdown links resolve to existing files inside the bundle
4. every concept under phases/, practices/, perspectives/, comparisons/ has a
   `# Citations` section containing at least one resolving link into /sources/

Frontmatter is parsed with a minimal flat parser: `key: value` and `key: [a, b]`
lines only (bundles produced by this repo use inline list syntax). This is
deliberately stricter than the OKF spec, so the checker is not a general-purpose
validator for third-party bundles.

Usage: python3 scripts/okf_check.py <bundle-dir>
Exits 0 and prints OK on success; exits 1 with one error per line otherwise.
"""
import re
import sys
from pathlib import Path

RESERVED = {"index.md", "log.md"}
CITATION_DIRS = {"phases", "practices", "perspectives", "comparisons"}
ALLOWED_TYPES = {"Source", "Practice", "Lifecycle Phase", "Perspective", "Comparison"}
LINK_RE = re.compile(r"\[[^\]]*\]\(([^)\s]+)\)")
KV_RE = re.compile(r"^([A-Za-z_][\w-]*):\s*(.*)$")
CITATIONS_HEADING_RE = re.compile(r"^#{1,3}\s+Citations\s*$", re.MULTILINE)
NEXT_HEADING_RE = re.compile(r"^#{1,3}\s+\S", re.MULTILINE)
MARKER_RE = re.compile(r"\[(\d+)\](?!\()")          # [3] but not [3](link)
CITATION_ENTRY_RE = re.compile(r"^\s*(\d+)\.\s", re.MULTILINE)


def split_frontmatter(text):
    """Return (frontmatter_text or None, body)."""
    if not text.startswith("---\n"):
        return None, text
    end = text.find("\n---\n", 4)
    if end == -1:
        return None, text
    return text[4:end], text[end + 5:]


def parse_frontmatter(fm_text):
    """Return a dict, or None if any non-blank line fails to parse."""
    data = {}
    for line in fm_text.splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        m = KV_RE.match(line)
        if not m:
            return None
        key, val = m.group(1), m.group(2).strip()
        if val.startswith("[") and val.endswith("]"):
            data[key] = [v.strip().strip("\"'") for v in val[1:-1].split(",") if v.strip()]
        else:
            data[key] = val.strip("\"'")
    return data


def iter_links(text):
    for target in LINK_RE.findall(text):
        if target.startswith(("http://", "https://", "mailto:", "#")):
            continue
        yield target.split("#")[0]


def check_bundle(root):
    root = Path(root).resolve()
    errors = []
    md_files = sorted(root.rglob("*.md"))
    if not md_files:
        return [f"{root}: no .md files found"]
    for path in md_files:
        rel = path.relative_to(root).as_posix()
        text = path.read_text(encoding="utf-8")
        fm_text, body = split_frontmatter(text)

        if path.name in RESERVED:
            if fm_text is not None:
                if rel == "index.md":
                    fm = parse_frontmatter(fm_text)
                    if fm is None or set(fm) != {"okf_version"}:
                        errors.append(
                            f"{rel}: bundle-root index.md frontmatter may declare only okf_version")
                else:
                    errors.append(f"{rel}: {path.name} must not have frontmatter")
        else:
            if fm_text is None:
                errors.append(f"{rel}: missing frontmatter")
            else:
                fm = parse_frontmatter(fm_text)
                if fm is None:
                    errors.append(f"{rel}: unparseable frontmatter")
                elif not str(fm.get("type", "")).strip():
                    errors.append(f"{rel}: missing or empty `type`")
                else:
                    node_type = str(fm.get("type")).strip()
                    if node_type not in ALLOWED_TYPES:
                        errors.append(
                            f"{rel}: unknown `type` {node_type!r} "
                            f"(allowed: {', '.join(sorted(ALLOWED_TYPES))})")
                    if (node_type == "Source"
                            and not str(fm.get("resource", "")).strip()):
                        errors.append(f"{rel}: Source node missing `resource` URL")

        for target in iter_links(body):
            if target.startswith("/"):
                resolved = (root / target.lstrip("/")).resolve()
            else:
                resolved = (path.parent / target).resolve()
            try:
                resolved.relative_to(root)
            except ValueError:
                errors.append(f"{rel}: link escapes bundle: {target}")
                continue
            if not resolved.exists():
                errors.append(f"{rel}: broken link: {target}")

        parts = Path(rel).parts
        if path.name not in RESERVED and parts and parts[0] in CITATION_DIRS:
            m = CITATIONS_HEADING_RE.search(body)
            if not m:
                errors.append(f"{rel}: missing # Citations section")
            else:
                section = body[m.end():]
                nxt = NEXT_HEADING_RE.search(section)
                if nxt:
                    section = section[:nxt.start()]
                cites = [t for t in iter_links(section) if t.startswith("/sources/")]
                if not cites:
                    errors.append(f"{rel}: # Citations has no /sources/ links")
                markers = set(MARKER_RE.findall(body[:m.start()]))
                entries = set(CITATION_ENTRY_RE.findall(section))
                missing = sorted(markers - entries, key=int)
                if missing:
                    errors.append(
                        f"{rel}: citation marker(s) with no numbered Citations entry: "
                        + ", ".join(f"[{n}]" for n in missing))

    for idx in md_files:
        if idx.name != "index.md" or idx.parent == root:
            continue
        _, idx_body = split_frontmatter(idx.read_text(encoding="utf-8"))
        listed = set()
        for target in iter_links(idx_body):
            if target.startswith("/"):
                listed.add((root / target.lstrip("/")).resolve())
            else:
                listed.add((idx.parent / target).resolve())
        for sib in sorted(idx.parent.glob("*.md")):
            if sib.name in RESERVED:
                continue
            if sib.resolve() not in listed:
                rel = idx.relative_to(root).as_posix()
                errors.append(f"{rel}: does not list {sib.name}")
    return errors


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    errors = check_bundle(sys.argv[1])
    for e in errors:
        print(e)
    if errors:
        print(f"FAIL: {len(errors)} error(s)")
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
