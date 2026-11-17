"""Real Git/process checks for the optional tutor diff probe; no model calls."""
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

MODULE = Path(__file__).resolve().parents[2] / "src-tauri/src/learning/v2/tutor_probe.py"
spec = importlib.util.spec_from_file_location("tutor_probe", MODULE)
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class TutorProbeTests(unittest.TestCase):
    def setUp(self):
        self.previous = Path.cwd()
        self.temp = tempfile.TemporaryDirectory(prefix="lynx-tutor-probe-")
        os.chdir(self.temp.name)
        self.git("init", "-q")
        self.git("config", "user.name", "Local Test")
        self.git("config", "user.email", "test@example.invalid")
        Path("main.c").write_text("int main(void) { return 0; }\n")
        Path("other.py").write_text("unselected_secret = 'original'\n")
        self.git("add", ".")
        self.git("commit", "-qm", "fixture")
        self.commit = self.git("rev-parse", "HEAD").strip()

    def tearDown(self):
        os.chdir(self.previous)
        self.temp.cleanup()

    def git(self, *args):
        return subprocess.check_output(["git", *args], text=True, stderr=subprocess.DEVNULL)

    def request(self):
        return {"sourcePaths": ["main.c"], "commit": self.commit}

    def test_only_selected_tracked_diff_and_environment_are_read(self):
        Path("main.c").write_text("int main(void) { return 42; }\n")
        Path("other.py").write_text("unselected_secret = 'must-not-send'\n")
        Path("new.py").write_text("untracked_secret = 'must-not-send'\n")
        result = probe.inspect(self.request())
        self.assertIn("return 42", result["diff"])
        self.assertNotIn("must-not-send", str(result))
        self.assertEqual(result["commit"], self.commit)
        self.assertFalse(result["untrackedIncluded"])
        self.assertTrue(result["compiler"])

    def test_hidden_traversal_untracked_and_changed_commit_are_rejected(self):
        for name in ["../main.c", "/tmp/main.c", ".env", ".ssh/key.py", "a/../main.c", "new.py"]:
            with self.subTest(name=name), self.assertRaises(ValueError):
                probe.inspect({"commit": self.commit, "sourcePaths": [name]})
        with self.assertRaises(ValueError):
            probe.inspect({"commit": "f" * 40, "sourcePaths": ["main.c"]})

    def test_new_staged_source_and_symlinks_do_not_expand_scope(self):
        Path("new.py").write_text("new = 1\n")
        self.git("add", "new.py")
        with self.assertRaises(ValueError):
            probe.inspect({"commit": self.commit, "sourcePaths": ["new.py"]})
        Path("main.c").unlink()
        Path("main.c").symlink_to("other.py")
        with self.assertRaises(ValueError):
            probe.inspect(self.request())

    def test_large_diff_fails_closed_instead_of_unbounded_context(self):
        Path("main.c").write_text("long line\n" * 10000)
        with self.assertRaisesRegex(ValueError, "output limit"):
            probe.inspect(self.request())

    def test_directory_with_source_suffix_cannot_expand_to_all_children(self):
        Path("directory.c").mkdir()
        Path("directory.c/secret.txt").write_text("private content\n")
        self.git("add", "directory.c")
        self.git("commit", "-qm", "directory fixture")
        with self.assertRaisesRegex(ValueError, "source file"):
            probe.inspect({"commit": self.git("rev-parse", "HEAD").strip(), "sourcePaths": ["directory.c"]})


if __name__ == "__main__":
    unittest.main()
