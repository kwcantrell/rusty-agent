#!/usr/bin/env python3
"""Hermetic tests for scripts/skills_lint.py.

Run directly: python3 scripts/test_skills_lint.py
"""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import skills_lint

GOOD_FM = """---
name: {name}
description: >-
  Does a thing. Use when testing the linter.
---

# body
"""


class SkillsLintTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        (self.root / ".agents" / "skills").mkdir(parents=True)
        (self.root / ".claude" / "skills").mkdir(parents=True)

    def add_skill(self, name, fm=None, link=True):
        d = self.root / ".agents" / "skills" / name
        d.mkdir()
        (d / "SKILL.md").write_text(fm if fm is not None else GOOD_FM.format(name=name))
        if link:
            (self.root / ".claude" / "skills" / name).symlink_to(
                Path("../../.agents/skills") / name
            )

    def test_clean_tree_passes(self):
        self.add_skill("alpha")
        self.assertEqual(skills_lint.lint(self.root), [])

    def test_missing_symlink(self):
        self.add_skill("alpha", link=False)
        self.assertTrue(any("missing symlink" in e for e in skills_lint.lint(self.root)))

    def test_missing_skill_md(self):
        (self.root / ".agents" / "skills" / "alpha").mkdir()
        self.assertTrue(any("missing SKILL.md" in e for e in skills_lint.lint(self.root)))

    def test_no_frontmatter(self):
        self.add_skill("alpha", fm="# no frontmatter here\n")
        self.assertTrue(any("no frontmatter" in e for e in skills_lint.lint(self.root)))

    def test_name_mismatch(self):
        self.add_skill("alpha", fm=GOOD_FM.format(name="beta"))
        self.assertTrue(any("!= directory" in e for e in skills_lint.lint(self.root)))

    def test_forbidden_name_word(self):
        self.add_skill("claude-helper", fm=GOOD_FM.format(name="claude-helper"))
        self.assertTrue(any("'claude'/'anthropic'" in e for e in skills_lint.lint(self.root)))

    def test_overlong_description(self):
        fm = "---\nname: alpha\ndescription: " + "x" * 1100 + "\n---\n"
        self.add_skill("alpha", fm=fm)
        self.assertTrue(any("max 1024" in e for e in skills_lint.lint(self.root)))

    def test_block_scalar_description_measured_joined(self):
        # >- folds lines with spaces; 600+600 chars + joiner must exceed 1024
        fm = ("---\nname: alpha\ndescription: >-\n  " + "x" * 600
              + "\n  " + "y" * 600 + "\n---\n")
        self.add_skill("alpha", fm=fm)
        self.assertTrue(any("max 1024" in e for e in skills_lint.lint(self.root)))

    def test_stray_entry_in_mirror(self):
        self.add_skill("alpha")
        (self.root / ".claude" / "skills" / "ghost").symlink_to(
            Path("../../.agents/skills/ghost")
        )
        self.assertTrue(any("stray entry" in e for e in skills_lint.lint(self.root)))

    def test_missing_mirror_dir(self):
        shutil.rmtree(self.root / ".claude" / "skills")
        self.add_skill("alpha", link=False)
        self.assertTrue(any("directory missing" in e for e in skills_lint.lint(self.root)))


if __name__ == "__main__":
    unittest.main()
