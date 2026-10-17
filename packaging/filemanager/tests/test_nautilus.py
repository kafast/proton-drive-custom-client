"""Tests der Nautilus-Erweiterung ohne Nautilus: Protokoll, Zwischenspeicher, Menü.

Tests for the Nautilus extension without Nautilus: protocol, cache, menu.

Aufruf / run: python3 -m unittest discover -s packaging/filemanager/tests
"""

import importlib.util
import json
import os
import socketserver
import tempfile
import threading
import unittest
from pathlib import Path
from unittest import mock

EXTENSION = Path(__file__).resolve().parent.parent / "nautilus" / "protonsync_nautilus.py"


def load_extension():
    spec = importlib.util.spec_from_file_location("protonsync_nautilus", EXTENSION)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ext = load_extension()


class FakeDaemon(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    """Unix socket that records every request and sends a fixed answer line."""

    daemon_threads = True

    def __init__(self, path, reply):
        self.requests = []
        self.reply = reply
        super().__init__(path, FakeHandler)


class FakeHandler(socketserver.StreamRequestHandler):
    def handle(self):
        line = self.rfile.readline()
        self.server.requests.append(json.loads(line))
        if self.server.reply is not None:
            self.wfile.write(self.server.reply)


class RequestTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.directory.name, "daemon.sock")

    def tearDown(self):
        self.directory.cleanup()

    def serve(self, reply):
        server = FakeDaemon(self.path, reply)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        return server

    def test_sends_versioned_request_and_returns_body(self):
        body = {"type": "path_status", "entries": []}
        server = self.serve(json.dumps({"kind": "response", "body": body}).encode() + b"\n")

        answer = ext.request({"type": "path_status", "paths": ["/a"]}, path=self.path)

        self.assertEqual(answer, body)
        self.assertEqual(
            server.requests,
            [{"version": 1, "request": {"type": "path_status", "paths": ["/a"]}}],
        )

    def test_missing_daemon_gives_none(self):
        self.assertIsNone(ext.request({"type": "status"}, path=self.path))

    def test_garbage_and_events_give_none(self):
        self.serve(b"not json\n")
        self.assertIsNone(ext.request({"type": "status"}, path=self.path))

    def test_event_instead_of_response_gives_none(self):
        self.serve(json.dumps({"kind": "event", "body": {}}).encode() + b"\n")
        self.assertIsNone(ext.request({"type": "status"}, path=self.path))

    def test_silent_daemon_times_out(self):
        self.serve(None)
        self.assertIsNone(ext.request({"type": "status"}, path=self.path, timeout=0.1))


class SocketPathTest(unittest.TestCase):
    def test_explicit_socket_wins(self):
        with mock.patch.dict(os.environ, {"PROTONSYNC_SOCKET": "/x.sock", "XDG_RUNTIME_DIR": "/r"}):
            self.assertEqual(ext.socket_path(), "/x.sock")

    def test_runtime_dir_then_system(self):
        with mock.patch.dict(os.environ, {"XDG_RUNTIME_DIR": "/r"}, clear=True):
            self.assertEqual(ext.socket_path(), "/r/protonsync/daemon.sock")
        with mock.patch.dict(os.environ, {}, clear=True):
            self.assertEqual(ext.socket_path(), "/run/protonsync/daemon.sock")


class LanguageTest(unittest.TestCase):
    def test_german_follows_the_locale(self):
        with mock.patch.dict(os.environ, {"LANG": "de_DE.UTF-8"}, clear=True):
            self.assertEqual(ext.text("free"), "Speicher freigeben")
        with mock.patch.dict(os.environ, {"LC_ALL": "en_US.UTF-8", "LANG": "de_DE.UTF-8"}, clear=True):
            self.assertEqual(ext.text("free"), "Free up space")
        with mock.patch.dict(os.environ, {}, clear=True):
            self.assertEqual(ext.text("pin"), "Always keep local")


class PathOfTest(unittest.TestCase):
    def test_file_uris_only(self):
        self.assertEqual(ext.path_of("file:///home/k/Proton/a%20b.txt"), "/home/k/Proton/a b.txt")
        self.assertIsNone(ext.path_of("sftp://host/file"))
        self.assertIsNone(ext.path_of("trash:///x"))


class StatusCacheTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.folder = self.directory.name
        for name in ("a.txt", "b.txt"):
            Path(self.folder, name).write_text("x")
        self.asked = []
        self.now = 100.0

    def tearDown(self):
        self.directory.cleanup()

    def ask(self, body):
        self.asked.append(body)
        entries = [{"path": path, "state": "synced"} for path in body["paths"]]
        return {"type": "path_status", "entries": entries}

    def cache(self):
        return ext.StatusCache(ask=self.ask, clock=lambda: self.now)

    def test_asks_once_per_folder_for_all_entries(self):
        cache = self.cache()
        a = os.path.join(self.folder, "a.txt")
        b = os.path.join(self.folder, "b.txt")

        self.assertEqual(cache.status(a)["state"], "synced")
        self.assertEqual(cache.status(b)["state"], "synced")

        self.assertEqual(len(self.asked), 1)
        self.assertEqual(self.asked[0], {"type": "path_status", "paths": sorted([a, b])})

    def test_asks_again_after_expiry_or_forget(self):
        cache = self.cache()
        a = os.path.join(self.folder, "a.txt")
        cache.status(a)
        self.now += ext.CACHE_SECONDS + 1
        cache.status(a)
        cache.forget()
        cache.status(a)
        self.assertEqual(len(self.asked), 3)

    def test_unreachable_daemon_gives_none(self):
        cache = ext.StatusCache(ask=lambda body: None, clock=lambda: self.now)
        self.assertIsNone(cache.status(os.path.join(self.folder, "a.txt")))

    def test_missing_folder_still_asks_for_the_path(self):
        cache = self.cache()
        path = os.path.join(self.folder, "gone", "c.txt")
        self.assertEqual(cache.status(path)["state"], "synced")
        self.assertEqual(self.asked[0]["paths"], [path])


class MenuTest(unittest.TestCase):
    def test_emblems(self):
        self.assertEqual(ext.emblem_for({"state": "online_only"}), "emblem-web")
        self.assertEqual(ext.emblem_for({"state": "conflict"}), "emblem-important")
        self.assertIsNone(ext.emblem_for({"state": "outside"}))
        self.assertIsNone(ext.emblem_for({"state": "folder"}))
        self.assertIsNone(ext.emblem_for(None))

    def test_actions_only_for_drive_entries(self):
        unpinned = {"in_drive": True, "pinned": False}
        pinned = {"in_drive": True, "pinned": True}
        synced = {"in_drive": False, "pinned": False}

        self.assertEqual(ext.menu_actions([unpinned]), ["pin", "free"])
        self.assertEqual(ext.menu_actions([pinned]), ["unpin", "free"])
        self.assertEqual(ext.menu_actions([pinned, unpinned]), ["pin", "unpin", "free"])
        self.assertEqual(ext.menu_actions([unpinned, synced]), [])
        self.assertEqual(ext.menu_actions([unpinned, None]), [])
        self.assertEqual(ext.menu_actions([]), [])

    def test_run_action_starts_the_helper_with_paths_as_arguments(self):
        with mock.patch.object(ext.subprocess, "Popen") as popen:
            ext.run_action("unpin", ["/m/$(evil)", "/m/b"])
        self.assertEqual(
            popen.call_args.args[0],
            ["protonsync-filemanager", "unpin", "/m/$(evil)", "/m/b"],
        )

    def test_missing_helper_does_not_break_nautilus(self):
        with mock.patch.object(ext.subprocess, "Popen", side_effect=FileNotFoundError):
            ext.run_action("pin", ["/m/a"])


if __name__ == "__main__":
    unittest.main()
