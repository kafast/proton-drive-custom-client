# Proxmox: Lese-Backup geteilter Ordner einrichten

> English: [proxmox-read-backup.md](proxmox-read-backup.md)

Diese Anleitung richtet auf einem Proxmox-Host eine kleine Instanz ein, die protonsync ohne
Oberfläche betreibt und ausgewählte Proton-Ordner **nur liest** und lokal sichert. Grundlage:

- ein **zweites Proton-Konto** („Lese-Konto“), mit dem das Hauptkonto die Ordner als
  **„Betrachter“** teilt (Handbuch 4.4, ADR 0022). Proton selbst verhindert dann, dass protonsync
  etwas ändert, auch bei einem Fehler im Programm oder wenn der Server übernommen wird;
- Sync-Paare mit `shared = true` und `direction = "backup_only"` (ADR 0019): Neue Versionen kommen
  dazu, alte wandern nach `_archiv/`, in Proton gelöschte Dateien bleiben lokal;
- Warnung bei fast voller Platte (ADR 0023) und Benachrichtigungen per ntfy/E-Mail (Handbuch 4.9);
- protonsync als gehärteter systemd-Dienst mit verschlüsselter Anmeldedatei (Handbuch 7.2).

protonsync ist inoffiziell und nicht von Proton unterstützt.

## Inhalt

1. [LXC oder VM?](#1-lxc-oder-vm)
2. [Proton vorbereiten](#2-proton-vorbereiten)
3. [Container in Proxmox anlegen](#3-container-in-proxmox-anlegen)
4. [protonsync installieren (ohne selbst zu bauen)](#4-protonsync-installieren-ohne-selbst-zu-bauen)
5. [Konfiguration](#5-konfiguration)
6. [Dienst einrichten und anmelden](#6-dienst-einrichten-und-anmelden)
7. [Probelauf, dann Dauerbetrieb](#7-probelauf-dann-dauerbetrieb)
8. [Prüfen, ob der Schutz greift](#8-prüfen-ob-der-schutz-greift)
9. [Betrieb: Speicherplatz, Aktualisieren, Sicherung der Sicherung](#9-betrieb)
10. [Fehlersuche](#10-fehlersuche)

## 1. LXC oder VM?

**Empfehlung: ein unprivilegierter LXC-Container mit Debian und protonsync als systemd-Dienst**
(Handbuch 7.2, „Variante B“), ohne Docker.

| | LXC (empfohlen) | VM |
|---|---|---|
| Ressourcen | rund 13 MB RAM im Leerlauf für protonsync, kein eigener Kernel | eigener Kernel, mindestens ein paar hundert MB RAM |
| FUSE | nicht nötig | nicht nötig |
| Docker | in LXC von Proxmox nicht empfohlen | möglich (Handbuch 7.1) |
| Abschottung | gut (unprivilegiert, eigener Benutzer, gehärtete Unit) | noch stärker |

FUSE braucht es nur für das Laufwerk „nur bei Bedarf“ (`[[mount]]`, nur Desktop). Ein Backup
speichert alles vollständig lokal, deshalb entfallen FUSE und damit alle Sonderrechte für den
Container. Eine VM lohnt sich nur, wenn du ohnehin Docker nutzen willst oder dem Container-Kernel
nicht traust; dann Handbuch 7.1 mit derselben Konfiguration wie unten.

## 2. Proton vorbereiten

1. **Zweites Proton-Konto anlegen** (das kostenlose genügt, z. B. `backup-leser@proton.me`).
   Für das Konto eine 2FA-App (TOTP) einrichten; ein Sicherheitsschlüssel (FIDO2) als einzige
   zweite Stufe wird nicht unterstützt.
2. **Im Hauptkonto** im Web jeden zu sichernden Ordner per E-Mail mit dem Lese-Konto teilen,
   Rolle **„Betrachter“** (nicht „Bearbeiter“).
3. **Mit dem Lese-Konto** im Web anmelden und unter „Mit mir geteilt“ jede Einladung annehmen.
   Die Namen der Ordner dort sind später die Werte für `remote`.

Das Passwort des Hauptkontos braucht der Server nie.

## 3. Container in Proxmox anlegen

In der Proxmox-Oberfläche „Create CT“ (oder per `pct create` in der Shell des Hosts):

| Einstellung | Wert |
|---|---|
| Vorlage | Debian 12 oder 13 (`local` → CT Templates → Templates → `debian-12-standard`) |
| Unprivileged container | **ja** |
| Nesting | **ja** (Standard bei neuen Containern; systemd braucht es für die Härtung der Unit) |
| Root-Disk | 4 GB |
| CPU / RAM / Swap | 1 Kern / 512 MB / 512 MB |
| Netzwerk | DHCP oder feste IP im Heimnetz |

**Speicher für das Backup:** am einfachsten ein eigener Mountpoint aus einem Proxmox-Speicher
(Container → Resources → Add → Mount Point), z. B. 500 GB auf `local-zfs`, Pfad `/backup`. Größe
großzügig wählen: `_archiv/` wird nie automatisch geleert. Mit „Backup“ angehakt sichert
`vzdump` den Mountpoint mit.

Per Shell auf dem Host (Container-Nummer 120 als Beispiel):

```sh
pct create 120 local:vztmpl/VORLAGE.tar.zst \
  --hostname protonsync --unprivileged 1 --features nesting=1 \
  --cores 1 --memory 512 --swap 512 --rootfs local-zfs:4 \
  --mp0 local-zfs:500,mp=/backup,backup=1 \
  --net0 name=eth0,bridge=vmbr0,ip=dhcp --onboot 1
pct start 120
pct enter 120
```

(`VORLAGE` durch den Namen aus `pveam list local` ersetzen, z. B. `debian-12-standard_…_amd64`.)

*Alternative:* einen vorhandenen Ordner des Hosts einbinden (`--mp0 /tank/backup,mp=/backup`).
Weil der Container unprivilegiert ist, gehören Dateien darin auf dem Host dann der UID
„100000 + UID im Container“; den Ordner auf dem Host deshalb entsprechend `chown`en (die UID des
Dienstbenutzers zeigt Schritt 4 mit `id protonsync`).

## 4. protonsync installieren (ohne selbst zu bauen)

Gebraucht werden nur die beiden statisch gelinkten Programme `protonsyncd` und `protonsync`
(laufen ohne weitere Bibliotheken). Bezugsquellen:

- **Nach dem ersten Release** (noch keines veröffentlicht): unter
  [Releases](https://github.com/kafast/proton-drive-custom-client/releases) die Datei
  `protonsync-VERSION-x86_64-linux-musl.tar.gz`. Direkt im Container:
  ```sh
  VERSION=0.1.0
  apt install -y curl
  curl -LO "https://github.com/kafast/proton-drive-custom-client/releases/download/v$VERSION/protonsync-$VERSION-x86_64-linux-musl.tar.gz"
  tar -xzf "protonsync-$VERSION-x86_64-linux-musl.tar.gz"
  cd "protonsync-$VERSION-x86_64-linux-musl"
  ```
- **Bis dahin aus der CI:** auf GitHub unter „Actions“ → „CI“ den neuesten grünen Lauf von `main`
  öffnen, unten bei „Artifacts“ `protonsync-x86_64-linux-musl` herunterladen (GitHub-Anmeldung
  nötig, die Datei verfällt nach 90 Tagen). Auf den Proxmox-Host kopieren und in den Container
  schieben:
  ```sh
  # auf dem Proxmox-Host
  pct push 120 protonsync-x86_64-linux-musl.zip /root/protonsync.zip
  # im Container
  apt install -y unzip && cd /root && unzip protonsync.zip
  ```
  Solange [PR #47](https://github.com/kafast/proton-drive-custom-client/pull/47) (geteilte Ordner,
  Speicherplatz-Warnung) nicht gemergt ist, enthält nur der CI-Lauf dieses PRs `shared = true`.

Dann im Container (als `root`), im Ordner mit den beiden Programmen:

```sh
apt install -y openssl
useradd --system --home-dir /var/lib/protonsync --shell /usr/sbin/nologin protonsync
install -m 0755 protonsyncd protonsync /usr/local/bin/
protonsync --version
install -d -m 0750 -o protonsync /etc/protonsync
sh -c 'umask 077; openssl rand -hex 32 > /etc/protonsync/key'
install -d -m 0750 -o protonsync -g protonsync /backup/proton
id protonsync          # UID merken, falls ein Host-Ordner eingebunden ist (Abschnitt 3)
```

`/etc/protonsync/key` ist der Hauptschlüssel der verschlüsselten Anmeldedatei. Er gehört `root`;
der Dienst bekommt ihn nur über ein systemd-Credential. Kopie an einem sicheren Ort aufbewahren
(z. B. im Passwortmanager); ohne ihn ist nach einer Wiederherstellung nur ein neues
`protonsync login` nötig, es geht nichts verloren.

## 5. Konfiguration

`/etc/protonsync/config.toml` anlegen (`nano /etc/protonsync/config.toml`). Alle Schlüssel mit
Erklärung: `protonsync config example` bzw. Handbuch 4.

```toml
[general]
dry_run = true                 # erst nur planen (Abschnitt 7)
secrets = "file"               # verschlüsselte Datei statt Schlüsselbund
trash_retention_days = 30
disk_check_minutes = 15        # Speicherplatz prüfen (ADR 0023)
disk_low_free_percent = 10     # „fast voll“
disk_full_free_percent = 2     # „voll“

# Ein [[pair]] je geteiltem Ordner. remote = Name unter „Mit mir geteilt“,
# optional mit Unterordner ("Familie/Fotos").
[[pair]]
id = "familie"
local = "/backup/proton/familie"
remote = "Familie"
shared = true
direction = "backup_only"

[[pair]]
id = "dokumente"
local = "/backup/proton/dokumente"
remote = "Dokumente"
shared = true
direction = "backup_only"

# Benachrichtigungen, z. B. per ntfy (App fürs Handy) oder E-Mail.
[notify]
language = "de"

[notify.webhook]
url = "https://ntfy.sh/ein-langes-geheimes-thema"
format = "text"
sync_failures = true
login_required = true
disk_space = true
weekly_report = true           # montags 8:00 eine Übersicht
```

Hinweise:

- `shared = true` erlaubt nur `backup_only` und `download_only`; ein anderes Paar lehnt der Daemon
  beim Start ab.
- `backup_only` statt `download_only`, damit Löschen oder Verschlüsseln im Hauptkonto die lokale
  Sicherung nicht trifft (alte Versionen landen in `_archiv/`, Handbuch 4.3).
- Statt ntfy geht auch E-Mail über `[notify.mail]` (Handbuch 4.9). Das SMTP-Passwort gehört in
  eine Datei mit Rechten 0600 oder in ein systemd-Credential, nie in die Konfiguration.
- `[web]` bleibt aus. Wer die Web-Oberfläche will: Handbuch 7.3. Wegen der gehärteten Unit kann der
  Daemon `/etc/protonsync/config.toml` nicht selbst ändern; Einstellungen dann in der Datei ändern
  und den Dienst neu starten.

## 6. Dienst einrichten und anmelden

Die Unit `protonsyncd.service` liegt im Release-Archiv bei (dann
`install -m 0644 protonsyncd.service /etc/systemd/system/`); beim CI-Artefakt fehlt sie, dann
direkt von GitHub laden:

```sh
curl -Lo /etc/systemd/system/protonsyncd.service \
  https://raw.githubusercontent.com/kafast/proton-drive-custom-client/main/packaging/systemd/protonsyncd.service
```

Dann eine Ergänzung anlegen:

```sh
systemctl edit protonsyncd
```

Im Editor zwischen den Kommentarzeilen eintragen, damit der Dienst den Backup-Ordner beschreiben darf:

```ini
[Service]
ReadWritePaths=/backup/proton
```

Starten und mit dem **Lese-Konto** anmelden:

```sh
systemctl daemon-reload
systemctl enable --now protonsyncd
systemctl status protonsyncd                     # „active (running)“

# Kurzbefehl für die CLI als Dienstbenutzer (in /root/.bashrc eintragen):
alias ps-cli='runuser -u protonsync -- env PROTONSYNC_SOCKET=/run/protonsync/daemon.sock protonsync'

ps-cli login                                     # Benutzername, Passwort, 2FA-Code des Lese-Kontos
ps-cli status
```

Gespeichert werden nur Sitzungsschlüssel, verschlüsselt in `/var/lib/protonsync`, nie das
Passwort. Nach einem Neustart meldet sich der Daemon selbst wieder an. Läuft die Sitzung dauerhaft
ab, kommt eine Benachrichtigung „Anmeldung nötig“; dann erneut `ps-cli login`.

## 7. Probelauf, dann Dauerbetrieb

```sh
ps-cli sync                         # plant nur: was würde heruntergeladen?
ps-cli sync --pair familie --apply  # einmal wirklich ausführen
ls -la /backup/proton/familie
```

Passt alles, in `/etc/protonsync/config.toml` `dry_run = false` setzen und
`systemctl restart protonsyncd`. Ab jetzt lädt der Daemon Änderungen selbst: über Proton-Ereignisse
(`event_poll_seconds`, Standard 60 s) und zur Sicherheit stündlich mit einem vollständigen Abgleich.

Protokoll ansehen: `journalctl -u protonsyncd -f`.

## 8. Prüfen, ob der Schutz greift

Zwei Punkte sind mit einem echten Konto **noch nicht geprüft** (Erprobung 3.13 und 3.14). Bitte
einmal durchspielen, bevor du dich auf die Einrichtung verlässt:

1. **Betrachter darf nicht löschen:** Im Web mit dem Lese-Konto versuchen, in einem geteilten Ordner
   eine Datei zu löschen oder umzubenennen. Proton muss das ablehnen. Klappt es doch, ist der Ordner
   als „Bearbeiter“ geteilt; Freigabe korrigieren.
2. **Änderungen kommen an:** Im Hauptkonto eine Datei im geteilten Ordner ändern. Nach spätestens
   etwa einer Minute steht im Protokoll ein Abgleich, die neue Version liegt in `/backup/proton/…`,
   die alte in `_archiv/`. Kommt sie erst nach bis zu einer Stunde, erreichen die Ereignisse des
   fremden Kontos das Lese-Konto nicht, und nur der stündliche Abgleich greift (bitte melden; dann
   ggf. `full_sync_minutes` verkleinern).
3. **Warnung bei voller Platte:** vorübergehend `disk_low_free_percent = 99`,
   `disk_full_free_percent = 98`, `disk_check_minutes = 1`, Dienst neu starten. Kurz danach muss
   die ntfy-/E-Mail-Meldung kommen. Werte zurücksetzen, neu starten.

Ergebnisse gern in `docs/erprobung.md` abhaken.

## 9. Betrieb

**Speicherplatz.** `_archiv/` wächst mit jeder geänderten Datei und wird nie automatisch geleert.
Die Warnung kommt bei unter 10 % bzw. 2 % freiem Platz. Bei voller Platte geht nichts kaputt
(Dateien werden erst vollständig geladen, dann an ihren Platz verschoben); es fehlen nur neue
Versionen, bis wieder Platz ist. Platz schaffen: alte Dateien in `_archiv/` löschen oder den
Mountpoint vergrößern (Proxmox: Container → Resources → Mount Point → Resize).

**Aktualisieren.** Neue Programme wie in Abschnitt 4 holen, dann:

```sh
install -m 0755 protonsyncd protonsync /usr/local/bin/
systemctl restart protonsyncd
```

Die Unit nur neu laden, wenn sie sich geändert hat (`systemctl edit` bleibt erhalten).

**Sicherung der Sicherung.** Weil protonsync im Container Schreibrechte auf `/backup` hat, schützen
zusätzlich Snapshots außerhalb des Containers, z. B. regelmäßige ZFS-Snapshots des Mountpoints auf
dem Host oder `vzdump` auf ein anderes Ziel (Proxmox Backup Server). So bleibt auch ein Stand
erhalten, falls im Container selbst etwas schiefgeht.

## 10. Fehlersuche

| Problem | Lösung |
|---|---|
| Dienst startet nicht, `status=226/NAMESPACE` | Im Container ist „Nesting“ aus. Container → Options → Features → Nesting einschalten, Container neu starten. |
| `status=243/CREDENTIALS` | Der Schlüssel fehlt unter `/etc/protonsync/key` oder ist nicht lesbar für `root`. |
| `Read-only file system` beim Schreiben in `/backup/…` | `ReadWritePaths=` fehlt oder zeigt auf einen anderen Ordner (Abschnitt 6). |
| `Permission denied` in `/backup/…` | Ordner gehört nicht `protonsync`. Bei eingebundenem Host-Ordner: auf dem Host `chown` auf 100000 + UID (Abschnitt 3). |
| `no folder "…" in "Shared with me"` | Einladung nicht angenommen, Name in `remote` anders geschrieben oder mit dem falschen Konto angemeldet. |
| CLI meldet „Daemon nicht erreichbar“ | `ps-cli` nutzen (Socket `/run/protonsync/daemon.sock`) und prüfen, ob der Dienst läuft. |
| Weitere | Handbuch 9. |
