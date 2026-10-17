"""Tests des Hilfsprogramms `protonsync-filemanager` und des Dolphin-Menüs.

Tests for the `protonsync-filemanager` helper and the Dolphin menu.

Aufruf / run: python3 -m unittest discover -s packaging/filemanager/tests
"""

import configparser
import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

FILEMANAGER = Path(__file__).resolve().parent.parent
HELPER = FILEMANAGER / "protonsync-filemanager"
DOLPHIN = FILEMANAGER / "dolphin" / "protonsync.desktop"

# Writes its arguments line by line to $CALLS; fails for paths containing "bad".
FAKE_CLI = """#!/bin/sh
printf '%s\\n' "$*" >> "$CALLS"
case "$*" in *bad*) echo "Fehler: $*" >&2; exit 3 ;; esac
"""
FAKE_NOTIFY = """#!/bin/sh
printf '%s\\n' "$@" >> "$NOTIFIED"
"""


def write_program(path, content):
    path.write_text(content)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


class HelperTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        root = Path(self.directory.name)
        self.calls = root / "calls"
        self.notified = root / "notified"
        bin_dir = root / "bin"
        bin_dir.mkdir()
        write_program(bin_dir / "protonsync", FAKE_CLI)
        write_program(bin_dir / "notify-send", FAKE_NOTIFY)
        self.env = {
            "PATH": f"{bin_dir}:/usr/bin:/bin",
            "CALLS": str(self.calls),
            "NOTIFIED": str(self.notified),
        }

    def tearDown(self):
        self.directory.cleanup()

    def run_helper(self, *args):
        return subprocess.run(
            ["sh", str(HELPER), *args],
            env=self.env,
            capture_output=True,
            text=True,
            check=False,
        )

    def lines(self, path):
        return path.read_text().splitlines() if path.exists() else []

    def test_pin_and_unpin_pass_all_paths_at_once(self):
        self.assertEqual(self.run_helper("pin", "/m/a b", "/m/c").returncode, 0)
        self.assertEqual(self.run_helper("unpin", "/m/a b").returncode, 0)
        self.assertEqual(self.lines(self.calls), ["pin /m/a b /m/c", "unpin /m/a b"])
        self.assertEqual(self.lines(self.notified), [])

    def test_free_runs_once_per_path_and_applies(self):
        self.assertEqual(self.run_helper("free", "/m/a", "/m/b").returncode, 0)
        self.assertEqual(self.lines(self.calls), ["free /m/a --apply", "free /m/b --apply"])

    def test_errors_become_a_notification_and_the_rest_still_runs(self):
        result = self.run_helper("free", "/m/bad", "/m/ok")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.lines(self.calls), ["free /m/bad --apply", "free /m/ok --apply"])
        notified = self.lines(self.notified)
        self.assertIn("protonsync", notified)
        self.assertIn("Fehler: free /m/bad --apply", notified)

    def test_paths_are_never_shell_text(self):
        marker = Path(self.directory.name) / "marker"
        self.run_helper("pin", f"/m/$(touch {marker})")
        self.assertFalse(marker.exists())

    def test_unknown_action_or_no_path_is_a_usage_error(self):
        self.assertEqual(self.run_helper("delete", "/m/a").returncode, 2)
        self.assertEqual(self.run_helper("pin").returncode, 2)
        self.assertEqual(self.lines(self.calls), [])

    def test_helper_is_executable(self):
        self.assertTrue(os.access(HELPER, os.X_OK))


class DolphinMenuTest(unittest.TestCase):
    def setUp(self):
        self.menu = configparser.ConfigParser(interpolation=None)
        self.menu.optionxform = str  # Keep keys like Name[de] unchanged.
        self.menu.read(DOLPHIN, encoding="utf-8")

    def test_is_a_kf6_service_menu(self):
        entry = self.menu["Desktop Entry"]
        self.assertEqual(entry["Type"], "Service")
        self.assertEqual(entry["X-KDE-ServiceTypes"], "KonqPopupMenu/Plugin")

    def test_every_action_is_bilingual_and_calls_the_helper(self):
        actions = [a for a in self.menu["Desktop Entry"]["Actions"].split(";") if a]
        self.assertEqual(actions, ["pin", "unpin", "free"])
        for action in actions:
            section = self.menu[f"Desktop Action {action}"]
            self.assertTrue(section["Name"])
            self.assertTrue(section["Name[de]"])
            self.assertEqual(section["Exec"], f"protonsync-filemanager {action} %F")


if __name__ == "__main__":
    unittest.main()
