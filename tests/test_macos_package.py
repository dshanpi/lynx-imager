import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    'packager', Path(__file__).resolve().parents[1] / 'scripts/package-macos-manual.py')
packager = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packager)


class MinimumOS(unittest.TestCase):
    def test_does_not_confuse_sdk_or_linker_with_minimum(self):
        self.assertEqual(packager.minimum_macos(
            '    minos 11.0\n      sdk 26.5\n  version 1267.0\n'), '11.0')

    def test_selects_highest_architecture_requirement(self):
        self.assertEqual(packager.minimum_macos('minos 11.0\nminos 26.0\n'), '26.0')

    def test_missing_metadata_cannot_pass(self):
        with self.assertRaises(ValueError):
            packager.minimum_macos('sdk 26.5\nversion 1267.0\n')

    def test_versions_are_numeric_and_padded(self):
        self.assertGreater(packager.version_tuple('26.0'), packager.version_tuple('15.9'))
        self.assertEqual(packager.version_tuple('26'), packager.version_tuple('26.0.0'))


if __name__ == '__main__':
    unittest.main()
