#!/usr/bin/env python3
"""Lint the Claude-facing skills trees (.agents/skills canonical, .claude/skills mirror).

Checks:
  1. Every .agents/skills/<name>/SKILL.md starts with a YAML frontmatter block
     whose `name` is legal (<=64 chars of [a-z0-9-], no "claude"/"anthropic")
     and equals the directory name, and whose `description` is non-empty and
     <=1024 chars (block scalars measured as the space-joined text).
  2. Every .agents/skills/<name>/ has a symlink .claude/skills/<name> resolving
     to it; no stray or dangling entries under .claude/skills/.

Stdlib-only (no PyYAML on this machine); frontmatter is parsed with regexes that
cover the plain and block-scalar (>-, >, |, |-) description forms used here.

Usage: python3 scripts/skills_lint.py [repo_root]   # exit 0 clean, 1 violations
"""
import re
import sys
from pathlib import Path

FM_RE = re.compile(r"\A---\n(.*?)\n---\n", re.S)
NAME_RE = re.compile(r"^name:\s*(\S+)\s*$", re.M)
DESC_RE = re.compile(r"^description:\s*(.*)(?:\n((?:[ ]{2,}.*\n?)*)|$)", re.M)
NAME_LEGAL = re.compile(r"[a-z0-9-]{1,64}\Z")
BLOCK_MARKERS = (">-", ">", "|", "|-")


def _frontmatter(text):
    m = FM_RE.match(text)
    return m.group(1) if m else None


def _description(block):
    m = DESC_RE.search(block)
    if not m:
        return None
    first, rest = m.group(1).strip(), m.group(2) or ""
    parts = [] if first in BLOCK_MARKERS else [first]
    parts += [ln.strip() for ln in rest.splitlines()]
    return " ".join(p for p in parts if p).strip()


def lint(root):
    errors = []
    canonical = root / ".agents" / "skills"
    mirror = root / ".claude" / "skills"
    skills = sorted(d for d in canonical.iterdir() if d.is_dir())

    for d in skills:
        rel = f".agents/skills/{d.name}"
        md = d / "SKILL.md"
        if not md.is_file():
            errors.append(f"{rel}: missing SKILL.md")
            continue
        block = _frontmatter(md.read_text(encoding="utf-8"))
        if block is None:
            errors.append(f"{rel}/SKILL.md: no frontmatter block at top of file")
        else:
            nm = NAME_RE.search(block)
            name = nm.group(1) if nm else ""
            if not NAME_LEGAL.fullmatch(name):
                errors.append(f"{rel}/SKILL.md: illegal or missing name {name!r}")
            elif "claude" in name or "anthropic" in name:
                errors.append(f"{rel}/SKILL.md: name may not contain 'claude'/'anthropic'")
            elif name != d.name:
                errors.append(f"{rel}/SKILL.md: name {name!r} != directory name {d.name!r}")
            desc = _description(block)
            if not desc:
                errors.append(f"{rel}/SKILL.md: missing or empty description")
            elif len(desc) > 1024:
                errors.append(f"{rel}/SKILL.md: description is {len(desc)} chars (max 1024)")
        link = mirror / d.name
        if not link.is_symlink():
            errors.append(f".claude/skills/{d.name}: missing symlink -> ../../{rel}")
        elif link.resolve() != d.resolve():
            errors.append(
                f".claude/skills/{d.name}: resolves to {link.resolve()}, expected {d.resolve()}"
            )

    if mirror.is_dir():
        expected = {d.name for d in skills}
        for entry in sorted(mirror.iterdir()):
            if entry.name not in expected:
                errors.append(
                    f".claude/skills/{entry.name}: stray entry (no matching .agents/skills/ dir)"
                )
    else:
        errors.append(".claude/skills/: directory missing")
    return errors


def main():
    root = (
        Path(sys.argv[1]).resolve()
        if len(sys.argv) > 1
        else Path(__file__).resolve().parent.parent
    )
    errors = lint(root)
    for e in errors:
        print(f"skills-lint: {e}", file=sys.stderr)
    n = sum(1 for d in (root / ".agents" / "skills").iterdir() if d.is_dir())
    print(f"skills-lint: {'FAIL' if errors else 'OK'} ({n} skills checked)")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
