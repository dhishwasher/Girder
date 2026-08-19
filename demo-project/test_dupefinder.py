"""Tests for dupefinder."""

import os
import tempfile
import unittest

from dupefinder import find_duplicates, hash_file


class HashFileTests(unittest.TestCase):
    def test_same_content_hashes_the_same(self):
        with tempfile.TemporaryDirectory() as folder:
            first = os.path.join(folder, "a.txt")
            second = os.path.join(folder, "b.txt")
            with open(first, "w") as handle:
                handle.write("identical")
            with open(second, "w") as handle:
                handle.write("identical")
            self.assertEqual(hash_file(first), hash_file(second))


class FindDuplicatesTests(unittest.TestCase):
    def test_groups_files_with_identical_content(self):
        with tempfile.TemporaryDirectory() as folder:
            for name in ("a.txt", "b.txt"):
                with open(os.path.join(folder, name), "w") as handle:
                    handle.write("same")
            with open(os.path.join(folder, "c.txt"), "w") as handle:
                handle.write("different")
            duplicates = find_duplicates(folder)
            self.assertEqual(len(duplicates), 1)
            self.assertEqual(len(list(duplicates.values())[0]), 2)

    def test_returns_nothing_when_all_files_differ(self):
        with tempfile.TemporaryDirectory() as folder:
            for index, name in enumerate(("a.txt", "b.txt", "c.txt")):
                with open(os.path.join(folder, name), "w") as handle:
                    handle.write(str(index))
            self.assertEqual(find_duplicates(folder), {})
