# Benutzerhandbuch protonsync

English version: [manual.md](manual.md)

Stand: Phase 3 (Platzhalter: FUSE-Laufwerk, Immer lokal halten, Freigabe). Dieses Handbuch beschreibt Installation, Einrichtung, Betrieb und Fehlersuche
auf dem Desktop (CachyOS/Arch) und auf einem Server (Container oder VM/LXC, z. B. auf Proxmox).

> **Wichtig:** protonsync ist ein inoffizieller Client und **nicht** von Proton entwickelt oder
> unterstützt. Er befindet sich im Alpha-Stadium. Verwende ihn nur mit Daten, von denen es eine
> weitere Sicherung gibt, und teste zuerst mit einem eigenen Testordner.

## Inhalt

1. [So funktioniert protonsync](#1-so-funktioniert-protonsync)
2. [Voraussetzungen](#2-voraussetzungen)
3. [Installation auf dem Desktop](#3-installation-auf-dem-desktop)
4. [Konfiguration](#4-konfiguration)
5. [Erster Start Schritt für Schritt](#5-erster-start-schritt-für-schritt)
6. [Bedienung im Alltag](#6-bedienung-im-alltag)
7. [Betrieb auf dem Server](#7-betrieb-auf-dem-server)
8. [Aktualisieren, Abmelden, Deinstallieren](#8-aktualisieren-abmelden-deinstallieren)
9. [Fehlersuche](#9-fehlersuche)
10. [Dateien und Pfade](#10-dateien-und-pfade)
11. [Sicherheit](#11-sicherheit)
12. [Für Entwickler: Tests und Live-Tests](#12-für-entwickler-tests-und-live-tests)

---

## 1. So funktioniert protonsync

protonsync besteht aus zwei Programmen:

| Programm | Aufgabe |
|---|---|
| `protonsyncd` | Der **Daemon** (Hintergrunddienst). Er hält die Anmeldung, beobachtet die Sync-Ordner, fragt Proton nach Änderungen und gleicht ab. |
| `protonsync` | Die **Kommandozeile** (CLI). Sie spricht über einen lokalen Socket mit dem Daemon. Ohne laufenden Daemon kann sie nur `config example` ausführen. |

Ein **Sync-Paar** verbindet einen lokalen Ordner mit einem Ordner in Proton Drive („Meine Dateien“).
Du kannst mehrere Paare anlegen. Jedes Paar hat eine Richtung:

- `two_way` (Standard): Änderungen auf beiden Seiten werden übernommen.
- `download_only`: Proton ist die Quelle, der lokale Ordner ist ein Spiegel (z. B. Backup-Kopie auf dem Server).
- `upload_only`: Der lokale Ordner ist die Quelle (z. B. ein Ordner mit Sicherungen).

Wann wird abgeglichen?

- **Lokal**: Der Daemon bemerkt Änderungen sofort (inotify) und gleicht nach einer kurzen Ruhezeit ab
  (`local_debounce_seconds`, Standard 3 Sekunden), damit nicht jede Teiländerung einzeln läuft.
- **Bei Proton**: Der Daemon fragt die Ereignisliste von Proton ab (`event_poll_seconds`, Standard 60,
  mindestens 30). Nur wenn Proton Änderungen meldet, wird abgeglichen. Das entspricht der Vorgabe von
  Proton, nicht ständig alles abzufragen.
- **Sicherheitsnetz**: Zusätzlich gibt es regelmäßig einen vollständigen Abgleich (`full_sync_minutes`, Standard 60).

Grundregeln, die immer gelten:

- **Nichts wird stillschweigend überschrieben.** Wurde eine Datei auf beiden Seiten geändert, entsteht
  ein Konflikt; beide Versionen bleiben erhalten (siehe [6.3](#63-konflikte)).
- **Nichts wird sofort gelöscht.** Löschungen landen zuerst in einem Papierkorb (lokal bzw. im
  Proton-Papierkorb) und werden erst nach Ablauf der Frist (Standard 30 Tage) endgültig entfernt.
- **Dry-Run zuerst.** Nach der Installation plant protonsync nur und zeigt an, was es tun würde.
  Erst wenn du `dry_run = false` setzt (oder einmalig `protonsync sync --apply` aufrufst), wird
  wirklich übertragen, umbenannt oder gelöscht.

## 2. Voraussetzungen

**Desktop**

- Linux mit systemd (getestet wird gegen CachyOS/Arch), KDE oder GNOME, Wayland oder X11.
- Ein Secret Service für die Anmeldedaten: GNOME Keyring oder KWallet (bei KDE/GNOME normalerweise
  schon aktiv).
- Zum Bauen: Rust über `rustup` sowie C-Compiler und `cmake` (für SQLite und die TLS-Bibliothek).
- Für das FUSE-Laufwerk (optional): das Paket `fuse3` (liefert `fusermount3`). Bei CachyOS/Arch
  meist schon installiert, sonst `sudo pacman -S fuse3`.

**Server**

- Variante A: Docker oder Podman (mit `compose`). Das Image bringt alles mit.
- Variante B: eine VM oder ein LXC-Container mit systemd; die statisch gelinkten Programme laufen
  ohne weitere Bibliotheken.

**Proton-Konto**

- Benutzername und Passwort, ggf. 2FA-App (TOTP) und ggf. das zweite Passwort
  („Mailbox-Passwort“ im Zwei-Passwort-Modus).
- Sicherheitsschlüssel (FIDO2) als **einzige** zweite Stufe werden nicht unterstützt; richte dann
  zusätzlich eine 2FA-App ein.

## 3. Installation auf dem Desktop

### 3.1 Werkzeuge installieren (CachyOS/Arch)

```sh
sudo pacman -S --needed base-devel cmake git rustup fuse3
rustup default stable
```

### 3.2 Quellcode holen und bauen

```sh
git clone https://github.com/kafast/proton-drive-custom-client.git ~/proton-drive-custom-client
cd ~/proton-drive-custom-client
cargo install --locked --path crates/daemon     # installiert protonsyncd nach ~/.cargo/bin
cargo install --locked --path crates/cli        # installiert protonsync nach ~/.cargo/bin
```

`rust-toolchain.toml` sorgt dafür, dass `rustup` automatisch eine passende Rust-Version verwendet.
Der erste Bau dauert einige Minuten. Prüfen:

```sh
protonsyncd --version
protonsync --version
```

Falls `command not found` erscheint, fehlt `~/.cargo/bin` im `PATH`. Ergänze in `~/.bashrc` bzw.
`~/.zshrc` die Zeile `export PATH="$HOME/.cargo/bin:$PATH"` und öffne ein neues Terminal.

### 3.3 Daemon als User-Dienst einrichten

```sh
mkdir -p ~/.config/systemd/user
cp packaging/systemd/protonsyncd.user.service ~/.config/systemd/user/protonsyncd.service
systemctl --user daemon-reload
```

Noch **nicht** starten: Zuerst die Konfiguration anlegen (nächster Abschnitt).

## 4. Konfiguration

### 4.1 Wo liegt die Datei?

| Umgebung | Pfad |
|---|---|
| Desktop | `~/.config/protonsync/config.toml` (bzw. `$XDG_CONFIG_HOME/protonsync/config.toml`) |
| Container | `/config/config.toml` |
| Server mit systemd | `/etc/protonsync/config.toml` (über `--config`) |

Der Pfad lässt sich mit der Umgebungsvariable `PROTONSYNC_CONFIG` oder `protonsyncd --config PFAD`
ändern. Fehlt die Datei, startet der Daemon mit Standardwerten und ohne Sync-Paare.

Eine kommentierte Vorlage erzeugst du mit:

```sh
mkdir -p ~/.config/protonsync
protonsync config example > ~/.config/protonsync/config.toml
```

Änderungen an der Datei wirken erst nach einem Neustart des Daemons
(`systemctl --user restart protonsyncd`).

### 4.2 Abschnitt `[general]`

| Schlüssel | Standard | Bedeutung |
|---|---|---|
| `dry_run` | `true` | `true`: automatische Abgleiche planen nur. `false`: sie führen aus. |
| `trash_retention_days` | `30` | Tage, die Gelöschtes im Papierkorb bleibt, bevor es endgültig entfernt wird. |
| `secrets` | `"keyring"` (Desktop-Build), `"file"` (Server-Build) | Wo die Anmeldung gespeichert wird, siehe [11](#11-sicherheit). |
| `event_poll_seconds` | `60` | Abstand der Abfrage der Proton-Ereignisse in Sekunden, mindestens `30`. |
| `full_sync_minutes` | `60` | Abstand des vollständigen Sicherheitsabgleichs in Minuten, größer als `0`. |
| `local_debounce_seconds` | `3` | Ruhezeit nach lokalen Änderungen, bevor abgeglichen wird. |
| `data_dir` | `~/.local/share/protonsync` | Datenverzeichnis (Zustandsdatenbank, verschlüsselte Geheimnisse). Auch über `PROTONSYNC_DATA_DIR`. |

### 4.3 Abschnitt `[[pair]]` (beliebig oft)

| Schlüssel | Pflicht | Bedeutung |
|---|---|---|
| `id` | ja | Kurzer Name des Paares, erlaubt sind `a-z`, `0-9`, `-`, `_`. Wird in der CLI verwendet. |
| `local` | ja | Lokaler Ordner als **absoluter** Pfad. Wird angelegt, falls er fehlt. |
| `remote` | ja | Ordner unterhalb von „Meine Dateien“ in Proton, z. B. `"Dokumente/Steuer"`. Leer (`""`) bedeutet „Meine Dateien“ selbst. Fehlende Ordner werden angelegt. |
| `direction` | nein | `"two_way"` (Standard), `"download_only"` oder `"upload_only"`. |

Regeln: Kennungen müssen eindeutig sein, und lokale Ordner dürfen sich nicht überlappen (kein Paar
darf in einem anderen liegen).

### 4.4 Beispiele

Desktop mit zwei Paaren:

```toml
[general]
dry_run = true

[[pair]]
id = "dokumente"
local = "/home/krischan/Proton/Dokumente"
remote = "Dokumente"

[[pair]]
id = "musik"
local = "/home/krischan/Musik"
remote = "Musik"
direction = "download_only"
```

Server als Backup-Spiegel von ganz „Meine Dateien“:

```toml
[general]
secrets = "file"
dry_run = false

[[pair]]
id = "spiegel"
local = "/backup/proton"
remote = ""
direction = "download_only"
```

### 4.5 Abschnitt `[[job]]`: Upload-Jobs (beliebig oft)

Ein Upload-Job kopiert Dateien aus einem lokalen Ordner regelmäßig nach Proton, z. B. die Backups
von Home Assistant. Anders als ein Sync-Paar arbeitet er nur in eine Richtung und räumt im Zielordner
nach einer Aufbewahrungsregel auf. Er funktioniert mit jedem Ordner, auch mit einer eingehängten
SMB- oder NFS-Freigabe.

| Schlüssel | Pflicht | Standard | Bedeutung |
|---|---|---|---|
| `id` | ja | | Kurzer Name, erlaubt sind `a-z`, `0-9`, `-`, `_`. |
| `source` | ja | | Quellordner als **absoluter** Pfad. Nur Dateien direkt darin zählen, keine Unterordner. |
| `pattern` | nein | `"*"` | Dateimuster für die Namen: `*` (beliebig viele Zeichen), `?` (ein Zeichen), `[abc]`, `[a-z]`, `[!x]`. Beispiel: `"*.tar"`. |
| `target` | ja | | Zielordner unterhalb von „Meine Dateien“, z. B. `"Backups/HomeAssistant"`. Wird angelegt, falls er fehlt. „Meine Dateien“ selbst ist nicht erlaubt. |
| `schedule` | ja | | Cron-Ausdruck (siehe unten) oder `"on_new_file"`. |
| `keep_last` | nein | alles | So viele der neuesten passenden Dateien bleiben im Ziel. |
| `max_age_days` | nein | alles | Passende Dateien, die älter sind, kommen in den Papierkorb. |
| `verify` | nein | `"hash"` | Prüfung nach dem Upload: `"hash"` (Größe und SHA-1-Prüfsumme) oder `"size"` (nur Größe). |
| `max_attempts` | nein | `5` | Versuche bei Fehlern, bevor der Job aufgibt und den Fehler meldet. |
| `settle_seconds` | nein | `60` | So viele Sekunden muss eine Datei unverändert sein, bevor sie hochgeladen wird. So landet kein halb geschriebenes Backup in Proton. |

**Zeitplan.** Ein Cron-Ausdruck hat fünf Felder: Minute, Stunde, Tag, Monat, Wochentag.
Erlaubt sind `*`, Listen (`1,15`), Bereiche (`1-5`), Schritte (`*/15`), Namen (`mon`, `jan`) und die
Kurzformen `@hourly`, `@daily`, `@weekly`, `@monthly`, `@yearly`. Sind Tag und Wochentag beide
eingeschränkt, genügt es, wenn eines von beiden passt (wie bei klassischem cron).

| Ausdruck | Bedeutung |
|---|---|
| `"0 3 * * *"` | jeden Tag um 3:00 Uhr |
| `"30 2 * * sun"` | sonntags um 2:30 Uhr |
| `"0 */6 * * *"` | alle sechs Stunden zur vollen Stunde |
| `"@daily"` | jeden Tag um 0:00 Uhr |

Die Uhrzeit ist die **Ortszeit** des Rechners, einschließlich Sommerzeit. Im Container stellst du sie
mit der Umgebungsvariable `TZ` ein (z. B. `TZ=Europe/Berlin`, siehe [7.1](#71-variante-a-container-dockerpodman)),
sonst gilt UTC. Fällt ein Termin in die Lücke der Zeitumstellung im März, läuft der Job direkt danach;
die doppelte Stunde im Oktober löst nur einen Lauf aus. Termine, die verpasst wurden, weil der Daemon
nicht lief, werden nicht nachgeholt.

Mit `"on_new_file"` läuft der Job beim Start des Daemons, kurz nach Änderungen im Quellordner und
außerdem alle fünf Minuten zur Sicherheit, weil Netzlaufwerke Änderungen oft nicht melden.
Proton wird dabei nur gefragt, wenn sich passende Dateien im Quellordner geändert haben.

**Ablauf eines Laufs.**

1. Passende Dateien im Quellordner suchen. Dateien in ihrer Ruhezeit (`settle_seconds`) warten.
2. Dateien, die im Ziel schon mit gleicher Größe (und bei `verify = "hash"` gleicher Prüfsumme)
   liegen, überspringen. Geänderte Dateien bekommen in Proton eine neue Version; die alte bleibt im
   Versionsverlauf von Proton.
3. Jeden Upload prüfen.
4. **Nur wenn alle Uploads geklappt haben**, die Aufbewahrung anwenden: Abgelaufene Dateien kommen
   in den Proton-Papierkorb und in den Papierkorb von protonsync (wiederherstellbar). Betroffen sind
   nur Dateien im Ziel, die zum Muster passen. Die **neueste Datei bleibt immer**, auch wenn sie älter
   als `max_age_days` ist; so bleibt mindestens ein Backup, falls die Quelle eine Weile nichts liefert.

Scheitert ein Lauf, versucht der Daemon es nach 1, 2, 4, 8, … Minuten erneut (höchstens eine
Stunde Abstand), bis `max_attempts` erreicht ist. Dann meldet er den Fehler im Protokoll und als
Ereignis für Oberflächen.

Bei `dry_run = true` planen auch die Jobs nur. Das Ergebnis siehst du mit `protonsync jobs`.

Beispiel für Home-Assistant-Backups, die auf einer SMB-Freigabe liegen (auf dem Host unter
`/mnt/ha-backups` eingehängt):

```toml
[[job]]
id = "home-assistant"
source = "/mnt/ha-backups"
pattern = "*.tar"
target = "Backups/HomeAssistant"
schedule = "0 4 * * *"       # täglich 4 Uhr, nach dem nächtlichen Backup von Home Assistant
keep_last = 14
max_age_days = 90
```

Tipp: Lege das Ziel eines Jobs nicht in einen Ordner, den auch ein Sync-Paar in beide Richtungen
abgleicht. Sonst lädt das Paar die Backups herunter, und Löschungen im Sync-Ordner würden auch die
Backups in Proton in den Papierkorb legen.

### 4.6 Abschnitt `[web]`: Web-Oberfläche

Standardmäßig aus. Schlüssel, Passwort-Hash und HTTPS stehen in [7.3](#73-web-oberfläche-optional).

### 4.7 Abschnitte `[[mount]]` und `[cache]`: FUSE-Laufwerk (nur Desktop)

Ein **Laufwerk** zeigt einen Proton-Ordner als virtuelles Verzeichnis. Alle Dateien und Ordner
erscheinen sofort, belegen aber keinen Platz: Den Inhalt lädt protonsync erst, wenn ein Programm
die Datei liest, und nur die gelesenen Teile (ein Video startet also sofort). Geladenes bleibt im
Zwischenspeicher, bis es freigegeben wird. Laufwerk und Sync-Paar sind zwei getrennte Betriebsarten;
du kannst beides gleichzeitig nutzen, nur nicht für denselben Ordner.

```toml
[[mount]]
id = "home"                                  # Kennung, wie bei Sync-Paaren
mountpoint = "/home/BENUTZER/ProtonDrive"    # absoluter Pfad, wird bei Bedarf angelegt
remote = ""                                  # Ordner in „Meine Dateien“; leer = alles
keep_local = false                           # Voreinstellung: false = nur bei Bedarf, true = immer lokal

[cache]                                      # gilt für alle Laufwerke
evict_after_days = 30                        # unbenutzte Dateien nach so vielen Tagen freigeben
max_size_gb = 10                             # Größenlimit des Zwischenspeichers (in GB, 1 GB = 10⁹ Byte)
```

| Schlüssel | Pflicht | Bedeutung |
|---|---|---|
| `id` | ja | Kennung aus Kleinbuchstaben, Ziffern, `-` und `_`; erscheint in `protonsync mounts` und im Papierkorb als `mount:ID` |
| `mountpoint` | ja | Absoluter Pfad, an dem das Laufwerk erscheint (am besten ein leeres Verzeichnis). Darf weder in einem Sync-Ordner oder anderen Laufwerk liegen noch eines enthalten. |
| `remote` | nein | Ordner in Proton; Standard ist die Wurzel von „Meine Dateien“ |
| `keep_local` | nein | `true`: alles wird im Hintergrund vollständig geladen und nie automatisch freigegeben |
| `evict_after_days` | nein | Standard 30 |
| `max_size_gb` | nein | Standard 10. Wird es überschritten, gibt protonsync die am längsten unbenutzten Dateien frei. |

Wie es sich verhält:

- **Eingehängt wird nach der Anmeldung**, ausgehängt beim Abmelden und beim Beenden des Daemons.
  `protonsync mounts` zeigt den Zustand.
- **Schreiben:** Neue und geänderte Dateien werden beim Schließen hochgeladen. Klappt das nicht
  (z. B. offline), bleiben sie vorgemerkt und werden regelmäßig (alle `event_poll_seconds`) erneut
  versucht. Hat sich die Datei inzwischen in Proton geändert, wird deine Fassung als
  **Konfliktkopie** daneben hochgeladen; nichts wird überschrieben.
- **Löschen** im Laufwerk legt die Datei in den Proton-Papierkorb (wiederherstellbar über
  `protonsync trash restore mount:ID NR`).
- **Änderungen in Proton** (z. B. vom Handy) erscheinen nach der nächsten Ereignisabfrage.
- **Freigabe:** Einmal pro Stunde gibt protonsync nach der Regel Speicher frei. Nie freigegeben
  werden angeheftete, geöffnete oder noch nicht hochgeladene Dateien. Im Dry-Run-Modus
  (`dry_run = true`) wird nur geplant, nichts entfernt.
- Der Zwischenspeicher liegt unter `<Daten>/cache/ID/` (nur für dich lesbar, Rechte 0700/0600).

Im Server-Build (Container) gibt es kein FUSE: Ein `[[mount]]` wird dort nicht eingehängt (Warnung im
Protokoll), Anheften und Freigeben funktionieren aber.

## 5. Erster Start Schritt für Schritt

Empfehlung für den ersten Versuch: Lege in Proton einen leeren Ordner an (z. B. `protonsync-test`),
trage ihn als `remote` ein und nimm einen neuen, leeren lokalen Ordner.

1. **Daemon starten**
   ```sh
   systemctl --user enable --now protonsyncd
   systemctl --user status protonsyncd        # sollte „active (running)“ zeigen
   ```
2. **Anmelden**
   ```sh
   protonsync login
   ```
   Abgefragt werden Benutzername, Passwort, ggf. der 2FA-Code und ggf. das Mailbox-Passwort.
   Die Eingaben erscheinen nicht auf dem Bildschirm. Gespeichert werden danach nur
   Sitzungsschlüssel, nie das Passwort. Nach einem Neustart meldet sich der Daemon selbst wieder an.
3. **Zustand ansehen**
   ```sh
   protonsync status
   ```
   Zeigt Konto, Dry-Run-Modus und die Sync-Paare mit ihrem letzten Lauf.
4. **Probelauf**
   ```sh
   protonsync sync
   ```
   Listet auf, was hoch- und heruntergeladen, angelegt oder in den Papierkorb gelegt würde,
   ohne etwas zu verändern.
5. **Einmal wirklich ausführen**
   ```sh
   protonsync sync --apply
   ```
6. **Dauerbetrieb einschalten**: in der Konfiguration `dry_run = false` setzen, dann
   ```sh
   systemctl --user restart protonsyncd
   ```
   Ab jetzt gleicht der Daemon selbstständig ab.

Soll der Daemon auch laufen, wenn du nicht angemeldet bist (z. B. nach dem Booten ohne Login):
`sudo loginctl enable-linger $USER`. Beachte, dass der Schlüsselbund dann gesperrt sein kann;
in diesem Fall eignet sich `secrets = "file"` (siehe [11](#11-sicherheit)) besser.

## 6. Bedienung im Alltag

### 6.1 Befehle

| Befehl | Was er tut |
|---|---|
| `protonsync status` | Konto, Modus, Sync-Paare, letzter Lauf |
| `protonsync sync` | Alle Paare **planen** (nichts ändern) |
| `protonsync sync --apply` | Alle Paare abgleichen |
| `protonsync sync --pair dokumente --apply` | Nur ein Paar abgleichen |
| `protonsync conflicts` | Offene Konflikte auflisten |
| `protonsync conflicts resolve PAAR NR keep-local` | Konflikt lösen, lokale Version behalten |
| `protonsync conflicts resolve PAAR NR keep-remote` | entfernte Version behalten |
| `protonsync conflicts resolve PAAR NR keep-both` | beide behalten (Konfliktkopie bleibt als eigene Datei) |
| `protonsync trash` | Papierkorb aller Paare auflisten |
| `protonsync trash restore PAAR NR` | Eintrag wiederherstellen |
| `protonsync trash purge` | Zeigen, welche abgelaufenen Einträge gelöscht würden |
| `protonsync trash purge --apply` | Abgelaufene Einträge endgültig löschen |
| `protonsync jobs` | Upload-Jobs mit letztem und nächstem Lauf |
| `protonsync jobs run JOB` | Einen Job **planen** (nichts ändern) |
| `protonsync jobs run JOB --apply` | Einen Job jetzt ausführen |
| `protonsync jobs history JOB [--limit N]` | Die letzten Läufe eines Jobs (Standard 10, höchstens 50) |
| `protonsync mounts` | FUSE-Laufwerke mit Zustand, Zwischenspeicher und angehefteten Pfaden |
| `protonsync pin PFAD` | „Immer lokal halten“ für eine Datei oder einen Ordner im Laufwerk (lädt im Hintergrund) |
| `protonsync unpin PFAD` | Zurück auf „nur bei Bedarf“ (wird später nach der Regel freigegeben) |
| `protonsync free [PFAD]` | Zeigen, was freigegeben würde (ohne Pfad: nach der Regel, mit Pfad: alles darunter) |
| `protonsync free [PFAD] --apply` | Speicher wirklich freigeben |
| `protonsync login` / `protonsync logout` | An- bzw. abmelden |
| `protonsync config example` | Beispielkonfiguration ausgeben |
| `protonsync --help`, `protonsync BEFEHL --help` | Hilfe |

Allgemeine Optionen:

- `--json`: Ausgabe als JSON, z. B. für Skripte: `protonsync --json status | jq .`
- `--socket PFAD`: anderen Daemon ansprechen (Standard siehe [10](#10-dateien-und-pfade)).
- `protonsync login --username NAME --password-stdin`: liest Passwort, 2FA-Code und
  Mailbox-Passwort zeilenweise von der Standardeingabe, z. B. für Automatisierung. Passwörter
  gehören nie in die Befehlszeile, weil sie dort in der Prozessliste sichtbar wären.

Die Sprache der Ausgaben richtet sich nach `LC_ALL`, `LC_MESSAGES` bzw. `LANG`
(Deutsch, sonst Englisch). Beispiel: `LANG=en_US.UTF-8 protonsync status`.

### 6.2 Exit-Codes (für Skripte)

| Code | Bedeutung |
|---|---|
| 0 | Erfolg |
| 1 | Allgemeiner Fehler |
| 2 | Falscher Aufruf (unbekannte Option o. Ä.) |
| 3 | Daemon nicht erreichbar |
| 4 | Nicht angemeldet |
| 5 | Anmeldung abgelehnt (falsches Passwort oder falscher Code) |
| 6 | Versionen von CLI und Daemon passen nicht zusammen |
| 7 | Teilweise fehlgeschlagen (mindestens ein Paar hatte Fehler, oder ein Job-Lauf ist gescheitert) |

### 6.3 Konflikte

Ein Konflikt entsteht, wenn dieselbe Datei seit dem letzten Abgleich **auf beiden Seiten** geändert
wurde, oder wenn sie auf einer Seite geändert und auf der anderen gelöscht wurde.

- Bei beidseitiger Änderung liegt die Proton-Version unter dem Originalnamen, die lokale Version
  daneben als **Konfliktkopie**, z. B. `Bericht (Konflikt 1760000000).pdf` (die Zahl ist der
  Zeitpunkt als Unix-Zeit). Es geht also nichts verloren.
- Der Konflikt bleibt in der Warteschlange, bis du entscheidest (`protonsync conflicts`).
  Das funktioniert auch auf dem Server, wo niemand sofort antworten kann.
- Beim Lösen mit `keep-local` oder `keep-remote` wandert die verworfene Version in den Papierkorb
  (wiederherstellbar). `keep-both` schließt den Konflikt nur; beide Dateien bleiben bestehen.
  Der nächste Abgleich überträgt das Ergebnis auf die andere Seite.

### 6.4 Papierkorb

- Wird eine Datei **in Proton** gelöscht, verschiebt protonsync die lokale Kopie in den lokalen
  Papierkorb `.protonsync-trash` im Sync-Ordner.
- Wird eine Datei **lokal** gelöscht, legt protonsync sie in Proton in den Proton-Papierkorb.
- `protonsync trash` zeigt beide Arten an; `restore` holt Einträge zurück.
- Nach `trash_retention_days` Tagen werden Einträge endgültig gelöscht. Der Daemon prüft das einmal
  täglich; im Dry-Run-Modus zeigt er nur an, was er löschen würde.

Papierkorbeinträge, die die Aufbewahrung eines Upload-Jobs angelegt hat, erscheinen unter
`job:ID`, z. B. `protonsync trash restore job:home-assistant 12`.

### 6.5 Laufwerk: Immer lokal halten und Speicher freigeben

`PFAD` ist ein normaler Pfad innerhalb des Laufwerks, relativ zum aktuellen Verzeichnis oder absolut:

```sh
protonsync pin ~/ProtonDrive/Steuern          # Ordner samt Inhalt immer lokal halten
protonsync unpin ~/ProtonDrive/Steuern/2019   # Ausnahme: dieser Unterordner nur bei Bedarf
protonsync free ~/ProtonDrive/Videos          # Probelauf: was würde frei?
protonsync free ~/ProtonDrive/Videos --apply  # wirklich freigeben
```

Die nächstgelegene Regel gilt: erst die Datei selbst, dann ihre Elternordner bis zum Laufwerk,
sonst `keep_local`. Freigegebene Dateien bleiben sichtbar und werden beim nächsten Öffnen neu
geladen. `free` überspringt angeheftete, geöffnete und noch nicht hochgeladene Dateien und nennt
ihre Anzahl unter „Behalten“.

### 6.6 Was wird nicht synchronisiert?

- Dateien und Ordner, deren Name mit `.protonsync-` beginnt (eigene Hilfsdateien, z. B. der Papierkorb
  und halb geschriebene Dateien während eines Downloads).
- Symbolische Links werden nicht verfolgt.
- Ignore-Muster folgen in einer späteren Phase.

## 7. Betrieb auf dem Server

### 7.1 Variante A: Container (Docker/Podman)

Das Image enthält nur `protonsyncd` und `protonsync`, ist statisch gelinkt und läuft als
Benutzer `nonroot` (UID 65532). Alles Nötige liegt in `packaging/container/`.

```sh
git clone https://github.com/kafast/proton-drive-custom-client.git
cd proton-drive-custom-client/packaging/container

# 1. Konfiguration anpassen (Sync-Paare). Pfade beziehen sich auf das Innere des Containers.
$EDITOR config.toml

# 2. Hauptschlüssel für die verschlüsselte Anmeldedatei erzeugen
openssl rand -hex 32 > protonsync-key
chmod 0400 protonsync-key
sudo chown 65532 protonsync-key

# 3. Sync-Ordner auf dem Host für den Container beschreibbar machen
sudo mkdir -p /srv/backup && sudo chown 65532 /srv/backup

# 4. Bauen und starten
docker compose up -d --build
docker compose logs -f              # Protokoll ansehen (Strg+C beendet nur die Anzeige)

# 5. Einmalig anmelden
docker compose exec protonsync protonsync login
docker compose exec protonsync protonsync status
```

Hinweise:

- In `compose.yaml` ist `/srv/backup` des Hosts als `/backup` im Container eingebunden. Passe das an
  deine Ordner an (z. B. eine SMB/NFS-Freigabe, die auf dem Host gemountet ist).
- Die Anmeldung liegt verschlüsselt im Volume `protonsync-data`, der Schlüssel getrennt davon als
  Docker-Secret. Bewahre `protonsync-key` sicher auf; ohne ihn musst du dich neu anmelden.
- Weitere Befehle laufen immer über `docker compose exec protonsync protonsync …`.
- `TZ` in `compose.yaml` legt die Ortszeit für die Zeitpläne der Upload-Jobs fest
  (Standard im Beispiel: `Europe/Berlin`).
- Mit Podman funktioniert dasselbe mit `podman compose`.
- Proxmox: Am einfachsten in einer kleinen VM oder einem LXC-Container mit Docker; alternativ
  Variante B direkt im LXC.

### 7.2 Variante B: systemd-Dienst (VM/LXC ohne Container)

Die Programme kannst du selbst bauen (`cargo build --release --no-default-features -p protonsync-daemon -p protonsync-cli`)
oder aus der CI holen: Jeder CI-Lauf enthält das Artefakt `protonsync-x86_64-linux-musl` mit beiden
statischen Programmen (GitHub → Actions → Lauf auswählen → Artifacts).

```sh
sudo useradd --system --home-dir /var/lib/protonsync --shell /usr/sbin/nologin protonsync
sudo install -m 0755 protonsyncd protonsync /usr/local/bin/
sudo install -d -m 0750 -o protonsync /etc/protonsync
sudo sh -c 'openssl rand -hex 32 > /etc/protonsync/key && chmod 0400 /etc/protonsync/key'
sudo $EDITOR /etc/protonsync/config.toml        # secrets = "file" und Sync-Paare eintragen
sudo cp packaging/systemd/protonsyncd.service /etc/systemd/system/
sudo systemctl daemon-reload
```

Der Dienst ist gehärtet und darf außer seinem Datenverzeichnis nichts schreiben. Gib die Sync-Ordner
deshalb in der Unit frei (Zeile `ReadWritePaths=` einkommentieren und anpassen), z. B. mit
`sudo systemctl edit protonsyncd`:

```ini
[Service]
ReadWritePaths=/srv/backup
```

Starten und anmelden:

```sh
sudo systemctl enable --now protonsyncd
sudo -u protonsync PROTONSYNC_SOCKET=/run/protonsync/daemon.sock protonsync login
sudo -u protonsync PROTONSYNC_SOCKET=/run/protonsync/daemon.sock protonsync status
journalctl -u protonsyncd -f
```

Der Schlüssel wird per `LoadCredential` nur dem Dienst übergeben und liegt nie im Datenverzeichnis.

### 7.3 Web-Oberfläche (optional)

Der Daemon kann eine schlanke Web-Oberfläche anbieten: Übersicht, Abgleich (planen oder ausführen),
Konflikte entscheiden, Papierkorb wiederherstellen und abgelaufene Einträge endgültig löschen
(erst planen, dann ausführen), Upload-Jobs starten mit Verlauf,
Einstellungen ansehen. Sie ist **standardmäßig aus**, braucht ein eigenes Passwort und kommt ohne
JavaScript aus. Sprache: Deutsch oder Englisch (Umschalter oben rechts, sonst nach Browser).

Was sie bewusst **nicht** kann: die Anmeldung bei Proton (dafür einmal `protonsync login` auf der
Kommandozeile) und das Ändern der Konfiguration (Datei bearbeiten, Daemon neu starten).

**1. Passwort-Hash erzeugen.** Das Passwort selbst steht nirgends; in die Konfiguration kommt nur
ein Argon2id-Hash. Es muss mindestens 12 Zeichen haben.

```sh
protonsyncd hash-password                          # Desktop / systemd
docker compose run --rm protonsync hash-password   # Container
```

**2. Abschnitt `[web]` eintragen:**

```toml
[web]
enabled = true
bind = "0.0.0.0:8470"           # Standard "127.0.0.1:8470": nur vom selben Rechner
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$…"   # Ausgabe von Schritt 1
allowed_networks = ["192.168.178.0/24"]               # optional, siehe unten
```

| Schlüssel | Standard | Bedeutung |
|---|---|---|
| `enabled` | `false` | Oberfläche einschalten |
| `bind` | `"127.0.0.1:8470"` | Adresse und Port. `0.0.0.0:8470` (bzw. `[::]:8470`) für das Heimnetz. |
| `password_hash` | – (Pflicht) | Hash aus `protonsyncd hash-password` |
| `allowed_networks` | private Netze | Nur aus diesen Netzen (CIDR) wird überhaupt geantwortet, sonst `403`. Standard: `127.0.0.0/8`, `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `169.254.0.0/16`, `::1/128`, `fc00::/7`. |
| `tls_cert`, `tls_key` | – | Zertifikatskette und privater Schlüssel (PEM) für HTTPS; nur zusammen. |
| `session_hours` | `12` | So lange bleibt man angemeldet. |

**3. Daemon neu starten** und `http://SERVER:8470/` (bzw. `https://…`) im Browser öffnen. Das
Protokoll meldet `Web-Oberfläche bereit` mit Adresse; kann sie nicht starten (Port belegt,
TLS-Dateien unbrauchbar), läuft der Daemon ohne sie weiter und protokolliert den Fehler.

**HTTPS.** Ohne TLS gehen Passwort und Inhalte unverschlüsselt durchs Netz; der Daemon warnt dann
beim Start, sobald `bind` nicht nur `127.0.0.1` ist. Zwei Wege:

- *Eigenes Zertifikat:* `tls_cert`/`tls_key` setzen. Für ein Heimnetz reicht ein selbst
  signiertes Zertifikat (der Browser fragt einmal nach):
  ```sh
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 825 \
    -subj "/CN=protonsync" -addext "subjectAltName=DNS:server.fritz.box,IP:192.168.178.10" \
    -keyout key.pem -out cert.pem
  chmod 0600 key.pem
  ```
  Der Daemon-Benutzer muss beide Dateien lesen können (Container: UID 65532, Dateien z. B. unter
  `/config` einbinden).
- *Reverse Proxy* (Caddy, nginx, Traefik) mit HTTPS davor: `bind = "127.0.0.1:8470"` lassen und den
  Proxy auf diese Adresse zeigen lassen. Die Oberfläche sieht dann nur die Adresse des Proxys;
  `X-Forwarded-For` wird absichtlich nicht ausgewertet. Der Zugriffsschutz nach Netzen gehört dann
  in den Proxy.

**Container.** Im Container `bind = "0.0.0.0:8470"` setzen (`127.0.0.1` wäre nur im Container
selbst erreichbar) und in `compose.yaml` die Zeile `ports` einkommentieren:
`"127.0.0.1:8470:8470"` für einen Proxy auf demselben Host, `"192.168.178.10:8470:8470"` für das
Heimnetz. Ohne IP vor dem ersten Doppelpunkt wäre der Port an **allen** Adressen des Hosts offen.

**Schutz im Überblick:** Anmelde-Cookie `HttpOnly`, `SameSite=Strict`, mit TLS `Secure`; jede
Aktion braucht eine CSRF-Kennung; nach 5 falschen Passwörtern ist die Adresse 5 Minuten gesperrt;
strenge Sicherheits-Header (u. a. Content-Security-Policy, kein Einbetten in fremde Seiten).
Sitzungen liegen nur im Speicher: Nach einem Neustart des Daemons meldet man sich neu an.

### 7.4 Ressourcen

Die Programme sind zusammen knapp 9 MB groß (gepackt). Für den Container ist ein Speicherlimit von
256 MB vorgesehen; im Leerlauf braucht der Daemon deutlich weniger. Genaue Messwerte folgen.

## 8. Aktualisieren, Abmelden, Deinstallieren

**Aktualisieren (Desktop)**

```sh
cd ~/proton-drive-custom-client && git pull
cargo install --locked --path crates/daemon && cargo install --locked --path crates/cli
systemctl --user restart protonsyncd
```

**Aktualisieren (Container)**: `git pull`, dann `docker compose up -d --build`.

**Abmelden**: `protonsync logout` löscht die gespeicherte Sitzung. Danach ist ein neues `login` nötig.

**Deinstallieren (Desktop)**

```sh
protonsync logout
systemctl --user disable --now protonsyncd
rm ~/.config/systemd/user/protonsyncd.service
cargo uninstall protonsync-daemon protonsync-cli
rm -r ~/.config/protonsync ~/.local/share/protonsync     # Konfiguration und Zustand
```

Deine Dateien in den Sync-Ordnern und in Proton bleiben dabei unverändert. Den Ordner
`.protonsync-trash` in einem Sync-Ordner kannst du danach von Hand löschen.

## 9. Fehlersuche

### 9.1 Protokolle

| Umgebung | Befehl |
|---|---|
| Desktop | `journalctl --user -u protonsyncd -f` |
| Server (systemd) | `journalctl -u protonsyncd -f` |
| Container | `docker compose logs -f` |

Mehr Details: Umgebungsvariable `RUST_LOG=debug` setzen, z. B. mit
`systemctl --user edit protonsyncd`:

```ini
[Service]
Environment=RUST_LOG=debug
```

Passwörter und Schlüssel erscheinen nie im Protokoll.

### 9.2 Häufige Probleme

| Meldung / Verhalten | Ursache und Lösung |
|---|---|
| `Daemon nicht erreichbar` (Exit 3) | Daemon läuft nicht: `systemctl --user status protonsyncd`. Oder CLI und Daemon nutzen verschiedene Sockets: `PROTONSYNC_SOCKET` prüfen. |
| `Nicht angemeldet` (Exit 4) | `protonsync login` ausführen. Kommt das nach einiger Zeit wieder, ist die Sitzung bei Proton abgelaufen oder wurde beendet (z. B. Passwortänderung). |
| `Anmeldung abgelehnt` (Exit 5) | Passwort oder 2FA-Code falsch, oder der Code war schon abgelaufen. Neu versuchen. |
| Sicherheitsschlüssel wird verlangt | Konten nur mit FIDO2-Schlüssel werden nicht unterstützt; 2FA-App zusätzlich einrichten. |
| Es passiert nichts | Ist `dry_run = true`? Dann wird nur geplant. `protonsync status` zeigt den Modus. |
| Job läuft zur falschen Uhrzeit | Die Zeitzone fehlt (dann gilt UTC). Im Container `TZ=Europe/Berlin` setzen; beim Start nennt das Protokoll die verwendete Zeitzone. |
| Job meldet „nicht gefunden“ für den Quellordner | Die Freigabe ist nicht eingehängt oder im Container nicht eingebunden. Eine nicht eingehängte, leere Freigabe meldet dagegen keinen Fehler, sondern einfach keine neuen Dateien. |
| Job lädt eine Datei immer wieder hoch | Die Datei wird bei jedem Backup neu geschrieben (gleicher Name, anderer Inhalt). Das ist gewollt: Proton bekommt jeweils eine neue Version. |
| `secrets = "keyring" ist in diesem Build nicht verfügbar` | Server-Build ohne Schlüsselbund: in der Konfiguration `secrets = "file"` setzen. |
| Fehler zum Hauptschlüssel | Bei `secrets = "file"` muss ein Schlüssel vorhanden sein: `PROTONSYNC_KEY_FILE` (Datei mit 64 Hex-Zeichen, Rechte höchstens 0600) oder systemd-Credential `protonsync-key`. |
| Schlüsselbund fragt nach Freigabe / ist gesperrt | Beim Desktop-Login entsperren; bei Betrieb ohne Desktop-Sitzung `secrets = "file"` verwenden. |
| `Permission denied` im Container | Sync-Ordner oder Schlüsseldatei gehören nicht UID 65532: `sudo chown 65532 …`. |
| `Ungültige Konfiguration …` | Die Meldung nennt den Fehler (z. B. relativer Pfad, doppelte `id`, unbekannter Schlüssel). Datei korrigieren, Daemon neu starten. |
| Web-Oberfläche: `403 Forbidden` | Deine Adresse liegt nicht in `allowed_networks`. Hinter Docker oder einem Proxy sieht die Oberfläche die Adresse der Brücke bzw. des Proxys. |
| Web-Oberfläche: Anmeldung gesperrt | 5 falsche Passwörter: 5 Minuten warten. Passwort vergessen: `protonsyncd hash-password` neu ausführen, `password_hash` ersetzen, Daemon neu starten. |
| Web-Oberfläche nicht erreichbar | Steht `bind` noch auf `127.0.0.1`? Im Container sind `0.0.0.0` und ein `ports`-Eintrag nötig. Das Protokoll sagt beim Start, ob und wo sie lauscht. |
| Laufwerk: `Transport endpoint is not connected` | Der Daemon wurde hart beendet. Beim nächsten Start räumt er den alten Mount selbst auf; sofort geht es mit `fusermount3 -u ~/ProtonDrive`. |
| Laufwerk wird nicht eingehängt („verbunden, nicht eingehängt“) | Das Protokoll nennt den Grund. Häufig: `fuse3` fehlt, oder an dem Pfad ist schon etwas eingehängt. Anheften und Freigeben funktionieren trotzdem. |
| Laufwerk ist leer | Nicht angemeldet: `protonsync mounts` zeigt „nicht verbunden“. `protonsync login` ausführen. |
| Datei im Laufwerk öffnet langsam | Der Inhalt wird beim ersten Lesen geladen. Für häufig genutzte Ordner `protonsync pin` verwenden. |
| Socket bereits belegt | Ein zweiter Daemon läuft schon. Nur einen Daemon pro Benutzer starten. |

## 10. Dateien und Pfade

| Was | Desktop | Server (systemd) | Container |
|---|---|---|---|
| Konfiguration | `~/.config/protonsync/config.toml` | `/etc/protonsync/config.toml` | `/config/config.toml` |
| Datenverzeichnis | `~/.local/share/protonsync` | `/var/lib/protonsync` | `/data` |
| Zustandsdatenbank | `<Daten>/state.sqlite` | ebenso | ebenso |
| Verschlüsselte Anmeldung (`secrets = "file"`) | `<Daten>/secrets/` | ebenso | ebenso |
| Socket | `$XDG_RUNTIME_DIR/protonsync/daemon.sock` | `/run/protonsync/daemon.sock` | `/data/run/daemon.sock` |
| Lokaler Papierkorb | `<Sync-Ordner>/.protonsync-trash` | ebenso | ebenso |
| Zwischenspeicher der Laufwerke | `<Daten>/cache/<ID>/` | – | – |

Umgebungsvariablen:

| Variable | Bedeutung |
|---|---|
| `PROTONSYNC_CONFIG` | Pfad der Konfigurationsdatei |
| `PROTONSYNC_DATA_DIR` | Datenverzeichnis |
| `PROTONSYNC_SOCKET` | Pfad des Sockets (Daemon und CLI) |
| `PROTONSYNC_KEY_FILE` | Datei mit dem Hauptschlüssel (64 Hex-Zeichen, Rechte ≤ 0600) |
| `PROTONSYNC_KEY` | Hauptschlüssel direkt (nur wenn nichts anderes geht) |
| `CREDENTIALS_DIRECTORY` | wird von systemd gesetzt; `protonsync-key` darin hat Vorrang |
| `RUST_LOG` | Protokollstufe (`error`, `warn`, `info`, `debug`, `trace`) |
| `LANG`, `LC_ALL`, `LC_MESSAGES` | Sprache der CLI-Ausgabe |

## 11. Sicherheit

- **Was wird gespeichert?** Nur Sitzungstokens und die daraus abgeleiteten Schlüssel-Passphrasen,
  nie das Passwort. Diese Daten sind trotzdem so wertvoll wie das Passwort: Wer sie hat, kann auf
  dein Proton Drive zugreifen.
- **Desktop (`secrets = "keyring"`)**: Ablage im Secret Service (GNOME Keyring/KWallet) unter dem
  Dienstnamen `protonsync`.
- **Server (`secrets = "file"`)**: Ablage im Datenverzeichnis, verschlüsselt mit AES-256-GCM
  (Dateirechte 0600, Verzeichnis 0700). Der Hauptschlüssel kommt getrennt davon aus einem
  systemd-Credential, einer Schlüsseldatei oder einer Umgebungsvariable. Lege Schlüssel und
  Datenverzeichnis nicht zusammen in ein Backup.
- **Web-Oberfläche**: standardmäßig aus. Wenn du sie einschaltest, nutze HTTPS (eigenes Zertifikat oder Reverse Proxy), sobald sie von anderen Rechnern erreichbar ist, und ein langes Passwort. Nie direkt ins Internet stellen; für Zugriff von unterwegs ein VPN (z. B. WireGuard) verwenden.
- **Zugriff auf den Daemon**: Der Socket hat die Rechte 0600; nur der Benutzer, unter dem der Daemon
  läuft, kann ihn bedienen.
- **Kennung gegenüber Proton**: protonsync meldet sich ehrlich als Drittanbieter-App
  `external-drive-protonsync@0.1.0-alpha`. Proton kann einzelne Versionen sperren.
- **Ende-zu-Ende-Verschlüsselung**: Ver- und Entschlüsselung passieren lokal wie bei den offiziellen
  Apps; Proton sieht keine Inhalte und keine Dateinamen.

## 12. Für Entwickler: Tests und Live-Tests

```sh
cargo test                                   # alle Tests, ohne Konto, ohne Netz
cargo test -p protonsync-application         # gezielt ein Crate
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p protonsync-fuse --test mount -- --ignored   # echtes FUSE-Laufwerk (braucht fuse3)
```

Live-Tests gegen ein echtes Konto laufen nur auf Wunsch und nur in einem eigenen Testordner in Proton
(Standard `protonsync-live-tests`, wird angelegt). Jeder Test arbeitet in einem frischen Unterordner.

```sh
export PROTONSYNC_LIVE_USERNAME='name@proton.me'
read -rs PROTONSYNC_LIVE_PASSWORD && export PROTONSYNC_LIVE_PASSWORD
# optional: PROTONSYNC_LIVE_DATA_PASSWORD (Mailbox-Passwort), PROTONSYNC_LIVE_TOTP (aktueller Code),
#           PROTONSYNC_LIVE_FOLDER (anderer Testordner)
cargo test -p protonsync-adapter-proton --test live_contract -- --ignored --test-threads=1
```

`read -rs` liest das Passwort ohne Anzeige, damit es nicht in der Shell-Historie landet.
Der 2FA-Code ist nur 30 Sekunden gültig; setze ihn direkt vor dem Aufruf.
