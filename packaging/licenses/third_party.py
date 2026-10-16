#!/usr/bin/env python3
"""Sammelt die Lizenztexte aller Bibliotheken, die in die Programme gelinkt werden.

MIT, Apache und ähnliche Lizenzen verlangen, dass ihr Text und der Copyright-Hinweis mit den
Programmen ausgeliefert werden. Das Skript liest `cargo metadata`, folgt ab den genannten
Paketen nur den normalen Abhängigkeiten (keine Test- und Bauabhängigkeiten) und schreibt je
Bibliothek Name, Version, Lizenzausdruck und die mitgelieferten Lizenzdateien.

Collects the license texts of all libraries linked into the programs. MIT, Apache and similar
licenses require their text and copyright notice to ship with the programs. The script reads
`cargo metadata`, follows only normal dependencies (no test or build dependencies) from the
given packages and writes name, version, license expression and the bundled license files for
each library.

Aufruf / usage:
  cargo metadata --format-version 1 --locked --filter-platform "$(rustc -vV | sed -n 's/^host: //p')" \
    | python3 packaging/licenses/third_party.py protonsync-daemon protonsync-cli > THIRD-PARTY-LICENSES.txt
"""

import json
import re
import sys
from pathlib import Path

LICENSE_FILE = re.compile(r"^(LICEN[CS]E|COPYING|NOTICE|COPYRIGHT)", re.IGNORECASE)
SEPARATOR = "=" * 78


def shipped_packages(metadata, roots):
    """Alle Pakete, die von `roots` aus über normale Abhängigkeiten erreichbar sind."""
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    pending = [pid for pid, package in packages.items() if package["name"] in roots]
    missing = set(roots) - {packages[pid]["name"] for pid in pending}
    if missing:
        raise SystemExit(f"unbekannte Pakete / unknown packages: {', '.join(sorted(missing))}")
    seen = set()
    while pending:
        pid = pending.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            if any(kind["kind"] is None for kind in dep["dep_kinds"]):
                pending.append(dep["pkg"])
    return sorted(
        (packages[pid] for pid in seen if packages[pid]["source"] is not None),
        key=lambda package: (package["name"], package["version"]),
    )


def license_files(package):
    folder = Path(package["manifest_path"]).parent
    files = [path for path in folder.iterdir() if path.is_file() and LICENSE_FILE.match(path.name)]
    if package.get("license_file"):
        files.append(folder / package["license_file"])
    return sorted(set(files))


def render(packages):
    lines = [
        "Lizenzen der mitgelieferten Bibliotheken / licenses of the bundled libraries",
        "",
        f"{len(packages)} Bibliotheken / libraries",
        "",
    ]
    for package in packages:
        lines += [
            SEPARATOR,
            f"{package['name']} {package['version']}",
            f"Lizenz / license: {package.get('license') or 'siehe Datei / see file'}",
        ]
        if package.get("authors"):
            lines.append(f"Autoren / authors: {', '.join(package['authors'])}")
        if package.get("repository"):
            lines.append(f"Quelle / source: {package['repository']}")
        files = license_files(package)
        if not files:
            lines.append("(keine Lizenzdatei im Paket / no license file in the package)")
        for path in files:
            lines += ["", f"--- {path.name} ---", path.read_text(encoding="utf-8", errors="replace").rstrip()]
        lines.append("")
    return "\n".join(lines) + "\n"


def main(argv):
    if len(argv) < 2:
        raise SystemExit(__doc__)
    metadata = json.load(sys.stdin)
    sys.stdout.write(render(shipped_packages(metadata, set(argv[1:]))))


if __name__ == "__main__":
    main(sys.argv)
