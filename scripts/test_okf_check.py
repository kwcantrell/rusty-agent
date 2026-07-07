import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import okf_check

VALID_SOURCE = """---
type: Source
title: Example
resource: https://example.com/post
---
# Summary
A claim.
"""


def write(root, rel, text):
    p = Path(root) / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text, encoding="utf-8")


def valid_bundle(root):
    write(root, "index.md",
          '---\nokf_version: "0.1"\n---\n# Bundle\n- [example](/sources/example.md)\n')
    write(root, "sources/index.md", "# Sources\n- [example](/sources/example.md)\n")
    write(root, "sources/example.md", VALID_SOURCE)
    write(root, "practices/evals.md",
          "---\ntype: Practice\ntitle: Evals\ntags: [building-agents]\n---\n"
          "Body claim [1].\n\n# Citations\n1. [example](/sources/example.md)\n")


class OkfCheckTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = self.tmp.name

    def tearDown(self):
        self.tmp.cleanup()

    def test_valid_bundle_passes(self):
        valid_bundle(self.root)
        self.assertEqual(okf_check.check_bundle(self.root), [])

    def test_missing_type_fails(self):
        valid_bundle(self.root)
        write(self.root, "sources/no_type.md", "---\ntitle: X\n---\nbody\n")
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("no_type.md" in e and "type" in e for e in errs))

    def test_missing_frontmatter_fails(self):
        valid_bundle(self.root)
        write(self.root, "sources/bare.md", "just a body\n")
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("bare.md" in e and "frontmatter" in e for e in errs))

    def test_broken_link_fails(self):
        valid_bundle(self.root)
        write(self.root, "sources/bad_link.md",
              "---\ntype: Source\n---\nSee [missing](/sources/nope.md)\n")
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("broken link" in e for e in errs))

    def test_external_links_ignored(self):
        valid_bundle(self.root)
        write(self.root, "sources/ext.md",
              "---\ntype: Source\nresource: https://example.com/ext\n---\n"
              "See [site](https://example.com/x) and [anchor](#schema)\n")
        write(self.root, "sources/index.md",
              "# Sources\n- [example](/sources/example.md)\n- [ext](/sources/ext.md)\n")
        self.assertEqual(okf_check.check_bundle(self.root), [])

    def test_index_missing_node_fails(self):
        valid_bundle(self.root)
        write(self.root, "sources/unlisted.md", VALID_SOURCE)
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("sources/index.md" in e and "unlisted.md" in e
                            for e in errs))

    def test_source_missing_resource_fails(self):
        valid_bundle(self.root)
        write(self.root, "sources/no_resource.md",
              "---\ntype: Source\ntitle: X\n---\n# Summary\nbody\n")
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("no_resource.md" in e and "resource" in e for e in errs))

    def test_unknown_type_fails(self):
        valid_bundle(self.root)
        write(self.root, "sources/typo.md",
              "---\ntype: Sorce\nresource: https://example.com/t\n---\nbody\n")
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("typo.md" in e and "Sorce" in e for e in errs))

    def test_non_root_index_frontmatter_fails(self):
        valid_bundle(self.root)
        write(self.root, "sources/index.md", "---\ntype: Index\n---\n# Sources\n")
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("sources/index.md" in e for e in errs))

    def test_root_index_extra_keys_fail(self):
        valid_bundle(self.root)
        write(self.root, "index.md", '---\nokf_version: "0.1"\ntype: Bundle\n---\n# B\n')
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("okf_version" in e for e in errs))

    def test_missing_citations_fails(self):
        valid_bundle(self.root)
        write(self.root, "practices/uncited.md",
              "---\ntype: Practice\n---\nA sourced claim with no citations.\n")
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("uncited.md" in e and "Citations" in e for e in errs))

    def test_citations_without_source_links_fail(self):
        valid_bundle(self.root)
        write(self.root, "practices/selfcite.md",
              "---\ntype: Practice\n---\nClaim.\n\n# Citations\n1. [me](/practices/evals.md)\n")
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("selfcite.md" in e for e in errs))

    def test_unresolved_citation_marker_fails(self):
        valid_bundle(self.root)
        write(self.root, "practices/dangling.md",
              "---\ntype: Practice\n---\nClaim [1] and claim [2].\n\n"
              "# Citations\n1. [example](/sources/example.md)\n")
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("dangling.md" in e and "marker" in e for e in errs))


if __name__ == "__main__":
    unittest.main()
