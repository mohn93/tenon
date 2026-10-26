#!/usr/bin/env python3
"""Tests of the SPDX expression check in licenses.py: python3 scripts/test_licenses.py"""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import licenses


class Satisfied(unittest.TestCase):
    def test_single_and_dual(self):
        self.assertTrue(licenses.satisfied("MIT"))
        self.assertTrue(licenses.satisfied("MIT/Apache-2.0"))
        self.assertTrue(licenses.satisfied("Apache-2.0 OR GPL-2.0-only"))
        self.assertTrue(licenses.satisfied("Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT"))

    def test_and_needs_every_term(self):
        self.assertTrue(licenses.satisfied("(MIT OR Apache-2.0) AND Unicode-3.0"))
        self.assertFalse(licenses.satisfied("MIT AND GPL-3.0-only"))
        self.assertFalse(licenses.satisfied("(MIT OR GPL-3.0-only) AND AGPL-3.0-only"))

    def test_unlisted(self):
        self.assertFalse(licenses.satisfied("GPL-3.0-only"))
        self.assertFalse(licenses.satisfied("LGPL-2.1-or-later OR GPL-2.0-only"))
        self.assertFalse(licenses.satisfied("Apache-2.0 WITH Some-exception"))

    def test_malformed(self):
        with self.assertRaises(ValueError):
            licenses.satisfied("(MIT OR Apache-2.0")

    def test_fence_outgrows_text(self):
        self.assertEqual(licenses.fence_for("a ``` b"), "````")
        self.assertEqual(licenses.fence_for("plain"), "```")


if __name__ == "__main__":
    unittest.main()
