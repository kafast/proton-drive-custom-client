"""protonsync-Erweiterung für Nautilus (GNOME Dateien), siehe ADR 0013.

Zeigt Embleme für Dateien in Laufwerken, Sync- und Fotoordnern und bietet im
Kontextmenü „Immer lokal halten“, „Nur bei Bedarf“ und „Speicher freigeben“.
Alle Arbeit macht der Daemon; diese Datei fragt ihn nur über seinen Socket.

Installation: nach ~/.local/share/nautilus-python/extensions/ kopieren
(Paket `python-nautilus` bzw. `nautilus-python`), `protonsync-filemanager`
in den PATH legen, dann `nautilus -q`.

protonsync extension for Nautilus (GNOME Files), see ADR 0013. Shows emblems
for files in drives, sync and photo folders and offers "Always keep local",
"On demand only" and "Free up space" in the context menu. The daemon does all
the work; this file only asks it through its socket.
"""

import json
import os
import socket
import subprocess
import time
from urllib.parse import unquote, urlparse

PROTOCOL_VERSION = 1
SOCKET_ENV = "PROTONSYNC_SOCKET"
SYSTEM_SOCKET = "/run/protonsync/daemon.sock"
# Nautilus wartet auf die Antwort; ein hängender Daemon darf es nicht blockieren.
TIMEOUT_SECONDS = 0.5
# So lange gilt eine Antwort für einen Ordner, bevor neu gefragt wird.
CACHE_SECONDS = 5.0
MAX_RESPONSE_BYTES = 4 * 1024 * 1024
# Gemeinsames Hilfsprogramm für die Menüpunkte (packaging/filemanager/).
HELPER = "protonsync-filemanager"

EMBLEMS = {
    "online_only": "emblem-web",
    "partial": "emblem-synchronizing",
    "local": "emblem-default",
    "pending_upload": "emblem-synchronizing",
    "synced": "emblem-default",
    "pending": "emblem-synchronizing",
    "conflict": "emblem-important",
}

TEXTS = {
    "pin": ("Immer lokal halten", "Always keep local"),
    "unpin": ("Nur bei Bedarf", "On demand only"),
    "free": ("Speicher freigeben", "Free up space"),
    "menu": ("protonsync", "protonsync"),
}


def german():
    """Deutsch, wenn die Sprache der Sitzung Deutsch ist (wie die CLI)."""
    for name in ("LC_ALL", "LC_MESSAGES", "LANG"):
        value = os.environ.get(name)
        if value:
            return value.lower().startswith("de")
    return False


def text(key):
    de, en = TEXTS[key]
    return de if german() else en


def socket_path():
    explicit = os.environ.get(SOCKET_ENV)
    if explicit:
        return explicit
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    if runtime:
        return os.path.join(runtime, "protonsync", "daemon.sock")
    return SYSTEM_SOCKET


def request(body, path=None, timeout=TIMEOUT_SECONDS):
    """Schickt eine Anfrage und liefert den Inhalt der Antwort; None bei Fehlern."""
    message = json.dumps({"version": PROTOCOL_VERSION, "request": body}) + "\n"
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(timeout)
            connection.connect(path or socket_path())
            connection.sendall(message.encode("utf-8"))
            data = b""
            while not data.endswith(b"\n") and len(data) < MAX_RESPONSE_BYTES:
                chunk = connection.recv(65536)
                if not chunk:
                    break
                data += chunk
    except OSError:
        return None
    try:
        answer = json.loads(data.decode("utf-8"))
    except ValueError:
        return None
    if answer.get("kind") != "response":
        return None
    return answer.get("body")


def path_of(uri):
    """Lokaler Pfad einer file://-Adresse; None für andere Adressen."""
    parsed = urlparse(uri)
    if parsed.scheme != "file":
        return None
    return unquote(parsed.path)


class StatusCache:
    """Merkt sich die Zustände je Ordner, damit Nautilus den Daemon nur einmal
    je Ordner fragt und nicht für jede Datei."""

    def __init__(self, ask=request, clock=time.monotonic):
        self._ask = ask
        self._clock = clock
        self._folders = {}

    def status(self, path):
        folder = os.path.dirname(path)
        cached = self._folders.get(folder)
        if cached is None or self._clock() - cached[0] > CACHE_SECONDS:
            cached = (self._clock(), self._load(folder, path))
            self._folders[folder] = cached
        return cached[1].get(path)

    def forget(self):
        self._folders.clear()

    def _load(self, folder, path):
        try:
            names = os.listdir(folder)
        except OSError:
            names = []
        paths = sorted({path, *(os.path.join(folder, name) for name in names)})
        answer = self._ask({"type": "path_status", "paths": paths})
        if not answer or answer.get("type") != "path_status":
            return {}
        return {entry["path"]: entry for entry in answer.get("entries", [])}


def emblem_for(entry):
    if not entry:
        return None
    return EMBLEMS.get(entry.get("state"))


def menu_actions(entries):
    """Welche Menüpunkte für die ausgewählten Dateien passen (nur in Laufwerken)."""
    in_drive = [entry for entry in entries if entry and entry.get("in_drive")]
    if not in_drive or len(in_drive) != len(entries):
        return []
    actions = []
    if any(not entry.get("pinned") for entry in in_drive):
        actions.append("pin")
    if any(entry.get("pinned") for entry in in_drive):
        actions.append("unpin")
    actions.append("free")
    return actions


def run_action(action, paths):
    """Führt die Aktion im Hintergrund über `protonsync-filemanager` aus, das
    Fehler als Benachrichtigung meldet (wie das Dolphin-Menü)."""
    try:
        subprocess.Popen(
            [HELPER, action, *paths],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
    except OSError:
        pass


try:
    import gi

    gi.require_version("Nautilus", "4.0")
    from gi.repository import GObject, Nautilus
except (ImportError, ValueError):  # Ohne Nautilus (z. B. in den Tests).
    Nautilus = None


if Nautilus is not None:

    class ProtonsyncExtension(GObject.GObject, Nautilus.InfoProvider, Nautilus.MenuProvider):
        def __init__(self):
            super().__init__()
            self._cache = StatusCache()

        def update_file_info(self, file):
            path = path_of(file.get_uri())
            if path is None:
                return
            emblem = emblem_for(self._cache.status(path))
            if emblem:
                file.add_emblem(emblem)

        def get_file_items(self, files):
            paths = [path_of(file.get_uri()) for file in files]
            if not paths or None in paths:
                return []
            actions = menu_actions([self._cache.status(path) for path in paths])
            if not actions:
                return []
            top = Nautilus.MenuItem(name="Protonsync::Menu", label=text("menu"))
            submenu = Nautilus.Menu()
            top.set_submenu(submenu)
            for action in actions:
                item = Nautilus.MenuItem(name="Protonsync::" + action, label=text(action))
                item.connect("activate", self._activate, action, paths)
                submenu.append_item(item)
            return [top]

        def _activate(self, _item, action, paths):
            run_action(action, paths)
            # Der neue Zustand soll beim nächsten Anzeigen sichtbar sein.
            self._cache.forget()
