# Erprobung mit dem echten Konto

English version: [below](#english)

Diese Checkliste führt Schritt für Schritt durch den ersten Test von protonsync mit deinem echten
Proton-Konto auf CachyOS (KDE oder GNOME). Hake jeden Schritt ab. Wenn etwas nicht wie beschrieben
aussieht, notiere Schritt und Ausgabe und schau in
[Abschnitt 5](#5-rückmeldung-wenn-etwas-schiefgeht). Erklärungen stehen im
[Handbuch](handbuch.md); hier stehen nur die Befehle und was du sehen solltest.

Grundregeln für die ganze Erprobung:

- Getestet wird nur in eigenen Testordnern: in Proton unter `protonsync-erprobung` (deine
  Handtests) und `protonsync-live-tests` (legen die automatischen Tests selbst an), lokal unter
  `~/protonsync-erprobung`. Lege dort nichts Wichtiges ab.
- `dry_run = true` bleibt eingeschaltet. Ausgeführt wird nur, wo ein Schritt ausdrücklich `--apply`
  nennt. Ausnahme: Schreiben und Löschen im FUSE-Laufwerk wirkt sofort (Schritt 3.8).
- Passwörter, 2FA-Codes und Tokens gehören nie in Dateien im Repository und nie in eine Rückmeldung.
- Die Befehle gehen davon aus, dass die Arbeitskopie unter `~/proton-drive-custom-client` liegt
  (wie im Handbuch). Liegt sie woanders, ersetze den Pfad.

## 1. Vorbereitung

- [ ] **1.1 Werkzeuge installieren**
  ([Handbuch 3.1](handbuch.md#31-werkzeuge-installieren-cachyosarch))
  ```sh
  sudo pacman -S --needed base-devel cmake git rustup fuse3
  sudo pacman -S --needed qt6-base qt6-declarative qt6-svg
  rustup default stable
  ```
  Fehler: `pacman` meldet Konflikte oder fehlende Pakete.

- [ ] **1.2 Aktuellen Stand holen**
  ```sh
  cd ~/proton-drive-custom-client
  git checkout main && git pull
  ```
  Noch keine Arbeitskopie? Dann klonen wie in
  [Handbuch 3.2](handbuch.md#32-quellcode-holen-und-bauen).

- [ ] **1.3 Bauen und installieren** ([Handbuch 3.2](handbuch.md#32-quellcode-holen-und-bauen))
  ```sh
  cargo install --locked --path crates/daemon
  cargo install --locked --path crates/cli
  cargo install --locked --path crates/gui-qt
  protonsyncd --version
  protonsync --version
  protonsync-tray --version
  ```
  Erwartet: drei Versionsnummern. Fehler: Abbruch beim Kompilieren (Ausgabe aufheben) oder
  `command not found` (dann fehlt `~/.cargo/bin` im `PATH`, siehe Handbuch 3.2).

- [ ] **1.4 User-Dienst einrichten, noch nicht starten**
  ([Handbuch 3.3](handbuch.md#33-daemon-als-user-dienst-einrichten))
  ```sh
  mkdir -p ~/.config/systemd/user
  cp packaging/systemd/protonsyncd.user.service ~/.config/systemd/user/protonsyncd.service
  systemctl --user daemon-reload
  ```

- [ ] **1.5 Testordner anlegen.** In Proton Drive im Browser einen leeren Ordner
  `protonsync-erprobung` anlegen (fehlende Unterordner legt protonsync selbst an). Lokal:
  ```sh
  mkdir -p ~/protonsync-erprobung
  ```

- [ ] **1.6 Konfiguration für die Erprobung anlegen** ([Handbuch 4](handbuch.md#4-konfiguration)).
  Die Datei braucht absolute Pfade, deshalb setzt die Shell `$HOME` ein. Gibt es schon eine
  Datei, sichere sie vorher mit
  `cp ~/.config/protonsync/config.toml ~/.config/protonsync/config.toml.bak`.
  ```sh
  mkdir -p ~/.config/protonsync
  cat > ~/.config/protonsync/config.toml <<EOF
  [general]
  dry_run = true
  trash_retention_days = 30

  [[pair]]
  id = "erprobung"
  local = "$HOME/protonsync-erprobung/sync"
  remote = "protonsync-erprobung/sync"
  direction = "two_way"
  EOF
  grep dry_run ~/.config/protonsync/config.toml
  ```
  Erwartet: `dry_run = true`. Damit planen alle automatischen Abgleiche, Aufbewahrungsregeln und
  Freigaben nur.

## 2. Automatische Live-Tests

- [ ] **2.1 Proton-Adapter gegen das echte Konto**
  ([Handbuch 12](handbuch.md#12-für-entwickler-tests-und-live-tests)). Erst bauen, damit der
  2FA-Code (30 Sekunden gültig) nicht während des Kompilierens abläuft:
  ```sh
  cd ~/proton-drive-custom-client
  cargo test -p protonsync-adapter-proton --test live_contract --no-run
  export PROTONSYNC_LIVE_USERNAME='name@proton.me'
  read -rs PROTONSYNC_LIVE_PASSWORD && export PROTONSYNC_LIVE_PASSWORD
  # nur im Zwei-Passwort-Modus:
  read -rs PROTONSYNC_LIVE_DATA_PASSWORD && export PROTONSYNC_LIVE_DATA_PASSWORD
  # nur mit 2FA, direkt vor dem Testlauf:
  read -r PROTONSYNC_LIVE_TOTP && export PROTONSYNC_LIVE_TOTP
  cargo test -p protonsync-adapter-proton --test live_contract -- --ignored --test-threads=1
  unset PROTONSYNC_LIVE_USERNAME PROTONSYNC_LIVE_PASSWORD PROTONSYNC_LIVE_DATA_PASSWORD \
    PROTONSYNC_LIVE_TOTP
  ```
  Erwartet: `test result: ok` mit `0 failed`. In Proton liegt danach `protonsync-live-tests` mit
  Unterordnern je Test; die kannst du im Browser löschen. Der Foto-Test liest nur.
  Fehler: jedes `FAILED`, eine Meldung wie `PROTONSYNC_LIVE_TOTP missing` (Variable fehlt) oder ein
  abgelehnter Login (Code abgelaufen: neuen Code setzen, Testlauf wiederholen).

- [ ] **2.2 Schlüsselbund (GNOME Keyring oder KWallet)**
  ```sh
  cargo test -p protonsync-adapter-secrets --lib -- --ignored round_trips_through_secret_service
  ```
  Erwartet: `1 passed`. KWallet oder GNOME Keyring fragen eventuell nach Freigabe; zustimmen.
  Fehler: Meldung, dass der Secret Service nicht erreichbar ist.

- [ ] **2.3 Echtes FUSE-Laufwerk (ohne Konto)**
  ```sh
  cargo test -p protonsync-fuse --test mount -- --ignored
  ```
  Erwartet: `test result: ok`. Fehler: Hinweis auf `/dev/fuse` oder `fusermount3` (dann fehlt
  `fuse3`, siehe 1.1).

- [ ] **2.4 Nichts Geheimes im Repository**
  ```sh
  git status --short
  ```
  Erwartet: keine neuen Dateien. Prüfe außerdem mit `env | grep PROTONSYNC_LIVE`, dass keine
  Variable mehr gesetzt ist (keine Ausgabe).

## 3. Von Hand: Ende-zu-Ende

- [ ] **3.1 Daemon starten und anmelden**
  ([Handbuch 5](handbuch.md#5-erster-start-schritt-für-schritt))
  ```sh
  systemctl --user enable --now protonsyncd
  systemctl --user status protonsyncd
  protonsync login
  protonsync status
  ```
  Erwartet: `active (running)`. `login` fragt Benutzername, Passwort, den 2FA-Code und im
  Zwei-Passwort-Modus das Mailbox-Passwort; nichts davon erscheint auf dem Bildschirm. `status`
  zeigt dein Konto, den Probelauf-Modus und das Paar `erprobung`.
  Fehler: Exit-Code 5 (abgelehnt), 3 (Daemon nicht erreichbar), siehe
  [Handbuch 9.2](handbuch.md#92-häufige-probleme). Den Exit-Code zeigt `echo $?`.

- [ ] **3.2 Anmeldung übersteht einen Neustart**
  ```sh
  systemctl --user restart protonsyncd
  protonsync status
  ```
  Erwartet: weiterhin angemeldet, ohne erneute Passwortabfrage. Fehler: `Nicht angemeldet` (Exit 4).

- [ ] **3.3 Erster Abgleich und lokale Änderung**
  ```sh
  protonsync sync
  echo "Hallo von protonsync" > ~/protonsync-erprobung/sync/hallo.txt
  protonsync sync
  protonsync sync --apply
  ```
  Erwartet: Der zweite Probelauf nennt den Upload von `hallo.txt`, ohne etwas zu tun; nach
  `--apply` liegt die Datei im Browser unter `protonsync-erprobung/sync`.

- [ ] **3.4 Änderung in Proton.** Im Browser eine kleine Datei in `protonsync-erprobung/sync`
  hochladen, etwa eine Minute warten (Ereignisabfrage), dann:
  ```sh
  protonsync sync
  protonsync sync --apply
  ls -l ~/protonsync-erprobung/sync
  ```
  Erwartet: Der Probelauf nennt den Download, danach liegt die Datei lokal mit richtiger Größe.
  Dann das Änderungsprotokoll ansehen ([Handbuch 6.11](handbuch.md#611-änderungsprotokoll)):
  ```sh
  protonsync changes --days 1
  ```
  Erwartet: je eine Zeile für den Upload aus 3.3 (Seite `Proton`) und den Download aus 3.4
  (Seite `lokal`), nichts aus den Probeläufen.

- [ ] **3.5 Konflikt erzeugen und lösen** ([Handbuch 6.3](handbuch.md#63-konflikte))
  1. Lokal ändern: `echo "lokal" >> ~/protonsync-erprobung/sync/hallo.txt`
  2. Im Browser eine andere Datei namens `hallo.txt` (anderer Inhalt) in denselben Ordner
     hochladen und „Ersetzen“ wählen. Eine Minute warten.
  3. Abgleichen und ansehen:
     ```sh
     protonsync sync --apply
     protonsync conflicts
     ls ~/protonsync-erprobung/sync
     protonsync conflicts resolve erprobung NR keep-both
     protonsync conflicts
     ```
  Erwartet: `hallo.txt` enthält die Proton-Fassung, daneben liegt `hallo (Konflikt …).txt` mit
  deiner lokalen Fassung. `NR` ist die Nummer aus der Liste; danach ist die Liste leer.
  Fehler: eine der beiden Fassungen fehlt. Das ist ein schwerer Fehler, bitte sofort melden.

- [ ] **3.6 Papierkorb und Wiederherstellen** ([Handbuch 6.4](handbuch.md#64-papierkorb))
  ```sh
  rm ~/protonsync-erprobung/sync/hallo.txt
  protonsync sync --apply
  protonsync trash
  protonsync trash restore erprobung NR
  protonsync sync --apply
  ```
  Erwartet: Nach dem Löschen liegt die Datei im Proton-Papierkorb, `trash` zeigt sie, nach dem
  Wiederherstellen ist sie in Proton und nach dem Abgleich wieder lokal.
  Danach umgekehrt: Datei im Browser löschen, eine Minute warten, `protonsync sync --apply`.
  Erwartet: Die lokale Kopie liegt in `~/protonsync-erprobung/sync/.protonsync-trash` und lässt
  sich ebenso mit `protonsync trash restore erprobung NR` zurückholen.
  `protonsync trash purge` zeigt nur, was abgelaufen wäre; **nicht** mit `--apply` ausführen.

- [ ] **3.7 Automatischer Abgleich (optional, kurz ohne Probelauf)**
  ```sh
  sed -i 's/^dry_run = true/dry_run = false/' ~/.config/protonsync/config.toml
  systemctl --user restart protonsyncd
  protonsync status
  echo "automatisch" > ~/protonsync-erprobung/sync/auto.txt
  ```
  Erwartet: `status` zeigt den Probelauf-Modus als aus; `auto.txt` erscheint nach wenigen Sekunden
  im Browser, eine im Browser hochgeladene Datei nach etwa einer Minute lokal. Danach zurück:
  ```sh
  sed -i 's/^dry_run = false/dry_run = true/' ~/.config/protonsync/config.toml
  systemctl --user restart protonsyncd
  ```

- [ ] **3.8 FUSE-Laufwerk**
  ([Handbuch 4.7](handbuch.md#47-abschnitte-mount-und-cache-fuse-laufwerk-nur-desktop),
  [6.5](handbuch.md#65-laufwerk-immer-lokal-halten-und-speicher-freigeben))
  ```sh
  cat >> ~/.config/protonsync/config.toml <<EOF

  [[mount]]
  id = "laufwerk"
  mountpoint = "$HOME/protonsync-erprobung/laufwerk"
  remote = "protonsync-erprobung/laufwerk"
  keep_local = false

  [cache]
  evict_after_days = 30
  max_size_gb = 1
  EOF
  systemctl --user restart protonsyncd
  protonsync mounts
  ```
  Erwartet: Das Laufwerk `laufwerk` ist eingehängt. Lade im Browser ein paar Dateien in
  `protonsync-erprobung/laufwerk` hoch (z. B. ein PDF, das du auch lokal hast, und ein Video).
  Eine Minute warten, dann (`DATEI.pdf` durch den echten Namen ersetzen):
  ```sh
  D=~/protonsync-erprobung/laufwerk
  ls -l "$D"
  protonsync file-status "$D"/*
  cmp ~/Downloads/DATEI.pdf "$D/DATEI.pdf"
  protonsync file-status "$D/DATEI.pdf"
  protonsync pin "$D/DATEI.pdf"
  protonsync mounts
  protonsync unpin "$D/DATEI.pdf"
  protonsync free "$D"
  protonsync free "$D" --apply
  protonsync file-status "$D/DATEI.pdf"
  ```
  Erwartet: Dateien erscheinen sofort mit voller Größe als `nur online (Platzhalter)`; `cmp` gibt
  nichts aus (Inhalt gleich); danach `lokal verfügbar`; `mounts` nennt den angehefteten Pfad;
  `free` plant erst, gibt dann frei, und die Datei ist wieder Platzhalter, aber sichtbar. Ein Video
  sollte im Player sofort starten.
  Schreiben und Löschen (wirkt sofort, ohne Probelauf):
  ```sh
  echo "neu" > "$D/neu.txt"
  rm "$D/neu.txt"
  protonsync trash
  protonsync trash restore mount:laufwerk NR
  ```
  Erwartet: `neu.txt` erscheint im Browser, liegt nach `rm` im Proton-Papierkorb und ist nach dem
  Wiederherstellen zurück. Fehler: `Transport endpoint is not connected` oder „nicht eingehängt“,
  siehe [Handbuch 9.2](handbuch.md#92-häufige-probleme).

- [ ] **3.9 Tray-App auf dem Desktop** ([Handbuch 6.6](handbuch.md#66-tray-app))
  ```sh
  protonsync-tray &
  ```
  Unter GNOME vorher die Erweiterung „AppIndicator and KStatusNotifierItem Support“ einschalten.
  Prüfen:
  - Symbol erscheint; der Tooltip nennt Zustand, Konto und Probelauf-Modus.
  - Klick öffnet das Statusfenster: Paar `erprobung` und Laufwerk `laufwerk`; **Ordner öffnen**
    öffnet den Dateimanager.
  - Menü (Rechtsklick) zeigt **Probelauf starten**; ein Klick ändert nichts an den Dateien.
  - Konfliktdialog: wie in 3.5 einen Konflikt mit einer neuen Datei `notiz.txt` erzeugen und
    `protonsync sync --apply` ausführen. Erwartet: Benachrichtigung, gelbes Symbol, in
    **Konflikte** dann **Beide behalten**; danach grünes Symbol und `protonsync conflicts` leer.
  - Sprache: unter **Einstellungen** auf Englisch und zurück auf Deutsch stellen; alle Texte
    wechseln. Für die CLI: `LANG=en_US.UTF-8 protonsync status`.
  - Daemon-Einstellungen (ADR 0021): unter **Einstellungen** beim Paar `erprobung` das Häkchen
    **Löschschutz** setzen. Erwartet: Meldung „Einstellung gespeichert“, `protonsync config show`
    zeigt `protect_remote = ja`, in der `config.toml` steht `protect_remote = true` und die
    Kommentare sind noch da. Häkchen wieder entfernen.
  - `systemctl --user restart protonsyncd`: Das Symbol wird kurz grau und verbindet sich nach
    wenigen Sekunden wieder.
  - **Beim Anmelden starten** anhaken, ab- und wieder anmelden: Die Tray-App startet von selbst.
  - Wenn möglich einmal unter Wayland und einmal unter X11 ansehen.

  Fehler: kein Symbol, abstürzende App oder englische Texte trotz Deutsch. Dann
  `protonsync-tray --check` im Terminal ausführen und die Ausgabe aufheben.

- [ ] **3.10 Dolphin oder Nautilus**
  ([Handbuch 6.9](handbuch.md#69-dateimanager-dolphin-und-nautilus)).
  Dateien von Hand installieren (aus `~/proton-drive-custom-client`):
  ```sh
  sudo pacman -S --needed libnotify
  install -Dm755 packaging/filemanager/protonsync-filemanager ~/.local/bin/protonsync-filemanager
  # Dolphin (KDE):
  install -Dm755 packaging/filemanager/dolphin/protonsync.desktop \
    ~/.local/share/kio/servicemenus/protonsync.desktop
  # Nautilus (GNOME):
  sudo pacman -S --needed python-nautilus
  install -Dm644 packaging/filemanager/nautilus/protonsync_nautilus.py \
    ~/.local/share/nautilus-python/extensions/protonsync_nautilus.py
  nautilus -q
  ```
  Erwartet: Rechtsklick auf eine Datei in `~/protonsync-erprobung/laufwerk` zeigt das Untermenü
  **protonsync** mit **Immer lokal halten**, **Nur bei Bedarf**, **Speicher freigeben**; die
  Wirkung zeigt `protonsync file-status`. Nautilus zeigt zusätzlich Embleme im Laufwerk und im
  Sync-Ordner (`F5` lädt neu). Fehler: kein Menü, oder eine Fehlermeldung als Benachrichtigung.

- [ ] **3.11 Kleiner Upload-Job**
  ([Handbuch 4.5](handbuch.md#45-abschnitt-job-upload-jobs-beliebig-oft))
  ```sh
  mkdir -p ~/protonsync-erprobung/job-quelle
  cd ~/protonsync-erprobung/job-quelle
  for n in 1 2 3; do echo "Backup $n" > backup-$n.tar; sleep 1; done
  cd -
  cat >> ~/.config/protonsync/config.toml <<EOF

  [[job]]
  id = "test-job"
  source = "$HOME/protonsync-erprobung/job-quelle"
  pattern = "*.tar"
  target = "protonsync-erprobung/job"
  schedule = "@daily"
  keep_last = 2
  settle_seconds = 5
  EOF
  systemctl --user restart protonsyncd
  protonsync jobs
  protonsync jobs run test-job
  ```
  Erwartet: `jobs` zeigt `test-job` mit nächstem Lauf; der Probelauf plant drei Uploads und nennt
  `backup-1.tar` als Datei, die die Aufbewahrung (`keep_last = 2`) in den Papierkorb legen würde.
  Erst wenn der Plan stimmt:
  ```sh
  protonsync jobs run test-job --apply
  protonsync jobs history test-job
  protonsync trash
  protonsync trash restore job:test-job NR
  ```
  Erwartet: drei Dateien im Browser, danach `backup-1.tar` im Papierkorb unter `job:test-job` und
  nach dem Wiederherstellen zurück. Fehler: Upload ohne vorherige Planung oder eine Löschung, die
  nicht im Papierkorb auftaucht.

- [ ] **3.12 Fotos, zuerst nur lesen**
  ([Handbuch 4.8](handbuch.md#48-abschnitt-photos-fotos-proton-photos),
  [6.8](handbuch.md#68-fotos-und-duplikate))
  ```sh
  cat >> ~/.config/protonsync/config.toml <<EOF

  [photos]
  folder = "$HOME/protonsync-erprobung/fotos"
  direction = "download_only"
  EOF
  systemctl --user restart protonsyncd
  protonsync photos
  protonsync photos sync
  protonsync photos duplicates
  ```
  Erwartet: Anzahl der Fotos, ein Plan nur mit Downloads und eine Liste möglicher Duplikate (nur
  Vorschlag, nichts wird gelöscht). Wenn genug Platz frei ist, `protonsync photos sync --apply`:
  Die Fotos landen unter `fotos/JJJJ/MM/`. Mit `download_only` wird in Proton nichts geändert.
  **Nicht** ausführen: `protonsync photos duplicates trash … --apply`.

## 4. Arch-Paket bauen und installieren

- [ ] **4.1 Installation per `cargo` entfernen** (Konfiguration und Anmeldung bleiben,
  [Handbuch 8](handbuch.md#8-aktualisieren-abmelden-deinstallieren)). Vorher die Tray-App über ihr
  Menü beenden.
  ```sh
  systemctl --user disable --now protonsyncd
  rm ~/.config/systemd/user/protonsyncd.service
  systemctl --user daemon-reload
  cargo uninstall protonsync-daemon protonsync-cli
  cargo uninstall protonsync-gui-qt
  rm -f ~/.config/autostart/protonsync-tray.desktop ~/.local/bin/protonsync-filemanager \
    ~/.local/share/kio/servicemenus/protonsync.desktop \
    ~/.local/share/nautilus-python/extensions/protonsync_nautilus.py
  ```

- [ ] **4.2 Paket bauen** ([Handbuch 3.5](handbuch.md#35-alternative-als-paket-archcachyos))
  ```sh
  cd ~/proton-drive-custom-client/packaging/arch
  makepkg -si
  sudo pacman -S --needed libnotify python-nautilus
  pacman -Qi protonsync-git
  command -v protonsync
  ```
  `makepkg -si` baut den Stand von `main` auf GitHub. Die eigene Arbeitskopie stattdessen:
  `PROTONSYNC_GIT="file://$HOME/proton-drive-custom-client" makepkg -si`.
  Erwartet: Paket `protonsync-git` installiert, `command -v` zeigt `/usr/bin/protonsync`.
  Fehler: `makepkg` bricht ab (die letzten 50 Zeilen der Ausgabe aufheben).

- [ ] **4.3 Mit dem Paket starten**
  ```sh
  systemctl --user enable --now protonsyncd
  systemctl --user status protonsyncd
  protonsync status
  protonsync mounts
  ```
  Erwartet: läuft, weiterhin angemeldet, Paar und Laufwerk wie vorher. Tray-App über das Startmenü
  („protonsync“) starten und **Beim Anmelden starten** neu anhaken. Dolphin- bzw. Nautilus-Menü
  (nach `nautilus -q`) funktioniert wie in 3.10.

- [ ] **4.4 Aufräumen, wenn alles erledigt ist.** Im Browser `protonsync-erprobung` und
  `protonsync-live-tests` löschen. Dann den Test-Abschnitt aus der Konfiguration entfernen (oder
  die Sicherung aus 1.6 zurückspielen), Daemon neu starten und lokal aufräumen:
  ```sh
  systemctl --user restart protonsyncd
  rm -r ~/protonsync-erprobung
  ```
  Abmelden bei Bedarf mit `protonsync logout`.

## 5. Rückmeldung, wenn etwas schiefgeht

Schick bitte diese Angaben (an Claude im Projekt oder als Issue im Repository):

- [ ] Schritt-Nummer, den genauen Befehl, was du erwartet und was du gesehen hast, Exit-Code
  (`echo $?` direkt danach).
- [ ] Versionen: `protonsync --version` und `protonsyncd --version` (beim Paket zusätzlich
  `pacman -Qi protonsync-git`).
- [ ] Zustand als JSON:
  ```sh
  protonsync --json status > ~/protonsync-status.json
  ```
  Bei Laufwerksfragen zusätzlich `protonsync --json mounts`.
- [ ] Protokoll des Daemons ([Handbuch 9.1](handbuch.md#91-protokolle)): live mit
  `journalctl --user -u protonsyncd -f`, als Datei für den Tag:
  ```sh
  journalctl --user -u protonsyncd --since today --no-pager > ~/protonsync-log.txt
  ```
  Bei schwer greifbaren Fehlern vorher `RUST_LOG=debug` setzen wie im Handbuch 9.1
  (`systemctl --user edit protonsyncd`), Fehler wiederholen, danach wieder entfernen.
- [ ] Bei der Tray-App: die Ausgabe von `protonsync-tray --check` bzw. von `protonsync-tray` im
  Terminal.

**Nie mitschicken:** Passwort, Mailbox-Passwort, 2FA-Codes, Werte der `PROTONSYNC_LIVE_*`-Variablen,
den Ordner `~/.local/share/protonsync/secrets/`, Einträge aus dem Schlüsselbund. Das Protokoll
enthält keine Passwörter, aber Dateinamen; lies es vor dem Senden durch und ersetze Namen, die du
nicht teilen möchtest.

---

## English

**Trial with the real account**

This checklist walks you step by step through the first test of protonsync with your real Proton
account on CachyOS (KDE or GNOME). Tick off every step. If something does not look as described,
note the step and the output and see [section 5](#5-feedback-when-something-goes-wrong).
Explanations are in the [manual](manual.md); this file only lists the commands and what you
should see.

Ground rules for the whole trial:

- Only test in dedicated test folders: in Proton under `protonsync-erprobung` (your manual tests)
  and `protonsync-live-tests` (created by the automated tests themselves), locally under
  `~/protonsync-erprobung`. Keep nothing important there.
- `dry_run = true` stays on. Changes are only carried out where a step explicitly says `--apply`.
  Exception: writing and deleting in the FUSE drive take effect immediately (step 3.8).
- Passwords, 2FA codes and tokens never go into files in the repository and never into a report.
- The commands assume the checkout lives in `~/proton-drive-custom-client` (as in the manual).
  If it lives elsewhere, replace the path.

### 1. Preparation

- [ ] **1.1 Install the tools** ([manual 3.1](manual.md#31-install-the-tools-cachyosarch))
  ```sh
  sudo pacman -S --needed base-devel cmake git rustup fuse3
  sudo pacman -S --needed qt6-base qt6-declarative qt6-svg
  rustup default stable
  ```
  Error: `pacman` reports conflicts or missing packages.

- [ ] **1.2 Get the current state**
  ```sh
  cd ~/proton-drive-custom-client
  git checkout main && git pull
  ```
  No checkout yet? Clone it as in [manual 3.2](manual.md#32-get-the-source-code-and-build).

- [ ] **1.3 Build and install** ([manual 3.2](manual.md#32-get-the-source-code-and-build))
  ```sh
  cargo install --locked --path crates/daemon
  cargo install --locked --path crates/cli
  cargo install --locked --path crates/gui-qt
  protonsyncd --version
  protonsync --version
  protonsync-tray --version
  ```
  Expected: three version numbers. Error: the build stops with a compile error (keep the output)
  or `command not found` (then `~/.cargo/bin` is missing from `PATH`, see manual 3.2).

- [ ] **1.4 Set up the user service, do not start it yet**
  ([manual 3.3](manual.md#33-set-up-the-daemon-as-a-user-service))
  ```sh
  mkdir -p ~/.config/systemd/user
  cp packaging/systemd/protonsyncd.user.service ~/.config/systemd/user/protonsyncd.service
  systemctl --user daemon-reload
  ```

- [ ] **1.5 Create the test folders.** In Proton Drive in the browser, create an empty folder
  `protonsync-erprobung` (protonsync creates missing subfolders itself). Locally:
  ```sh
  mkdir -p ~/protonsync-erprobung
  ```

- [ ] **1.6 Create the configuration for the trial** ([manual 4](manual.md#4-configuration)).
  The file needs absolute paths, so the shell fills in `$HOME`. If a file already exists, back it
  up first with `cp ~/.config/protonsync/config.toml ~/.config/protonsync/config.toml.bak`.
  ```sh
  mkdir -p ~/.config/protonsync
  cat > ~/.config/protonsync/config.toml <<EOF
  [general]
  dry_run = true
  trash_retention_days = 30

  [[pair]]
  id = "erprobung"
  local = "$HOME/protonsync-erprobung/sync"
  remote = "protonsync-erprobung/sync"
  direction = "two_way"
  EOF
  grep dry_run ~/.config/protonsync/config.toml
  ```
  Expected: `dry_run = true`. With this, all automatic syncs, retention rules and evictions only
  plan.

### 2. Automated live tests

- [ ] **2.1 Proton adapter against the real account**
  ([manual 12](manual.md#12-for-developers-tests-and-live-tests)). Build first, so that the
  2FA code (valid for 30 seconds) does not expire while compiling:
  ```sh
  cd ~/proton-drive-custom-client
  cargo test -p protonsync-adapter-proton --test live_contract --no-run
  export PROTONSYNC_LIVE_USERNAME='name@proton.me'
  read -rs PROTONSYNC_LIVE_PASSWORD && export PROTONSYNC_LIVE_PASSWORD
  # only in two-password mode:
  read -rs PROTONSYNC_LIVE_DATA_PASSWORD && export PROTONSYNC_LIVE_DATA_PASSWORD
  # only with 2FA, right before the test run:
  read -r PROTONSYNC_LIVE_TOTP && export PROTONSYNC_LIVE_TOTP
  cargo test -p protonsync-adapter-proton --test live_contract -- --ignored --test-threads=1
  unset PROTONSYNC_LIVE_USERNAME PROTONSYNC_LIVE_PASSWORD PROTONSYNC_LIVE_DATA_PASSWORD \
    PROTONSYNC_LIVE_TOTP
  ```
  Expected: `test result: ok` with `0 failed`. Afterwards Proton contains `protonsync-live-tests`
  with one subfolder per test; you can delete them in the browser. The photo test only reads.
  Error: any `FAILED`, a message like `PROTONSYNC_LIVE_TOTP missing` (variable not set) or a
  rejected login (code expired: set a new code, run the tests again).

- [ ] **2.2 Keyring (GNOME Keyring or KWallet)**
  ```sh
  cargo test -p protonsync-adapter-secrets --lib -- --ignored round_trips_through_secret_service
  ```
  Expected: `1 passed`. KWallet or GNOME Keyring may ask for permission; allow it.
  Error: a message that the Secret Service cannot be reached.

- [ ] **2.3 Real FUSE drive (no account needed)**
  ```sh
  cargo test -p protonsync-fuse --test mount -- --ignored
  ```
  Expected: `test result: ok`. Error: a hint about `/dev/fuse` or `fusermount3` (then `fuse3` is
  missing, see 1.1).

- [ ] **2.4 Nothing secret in the repository**
  ```sh
  git status --short
  ```
  Expected: no new files. Also check with `env | grep PROTONSYNC_LIVE` that no variable is still
  set (no output).

### 3. By hand: end to end

- [ ] **3.1 Start the daemon and log in** ([manual 5](manual.md#5-first-start-step-by-step))
  ```sh
  systemctl --user enable --now protonsyncd
  systemctl --user status protonsyncd
  protonsync login
  protonsync status
  ```
  Expected: `active (running)`. `login` asks for username, password, the 2FA code and, in
  two-password mode, the mailbox password; none of it is shown on screen. `status` shows your
  account, dry-run mode and the pair `erprobung`.
  Error: exit code 5 (rejected), 3 (daemon not reachable), see
  [manual 9.2](manual.md#92-common-problems). `echo $?` shows the exit code.

- [ ] **3.2 The login survives a restart**
  ```sh
  systemctl --user restart protonsyncd
  protonsync status
  ```
  Expected: still logged in, without asking for the password again. Error: not logged in (exit 4).

- [ ] **3.3 First sync and a local change**
  ```sh
  protonsync sync
  echo "Hallo von protonsync" > ~/protonsync-erprobung/sync/hallo.txt
  protonsync sync
  protonsync sync --apply
  ```
  Expected: the second dry run lists the upload of `hallo.txt` without doing it; after `--apply`
  the file shows up in the browser under `protonsync-erprobung/sync`.

- [ ] **3.4 A change in Proton.** In the browser, upload a small file into
  `protonsync-erprobung/sync`, wait about a minute (event polling), then:
  ```sh
  protonsync sync
  protonsync sync --apply
  ls -l ~/protonsync-erprobung/sync
  ```
  Expected: the dry run lists the download, afterwards the file is local with the right size.

- [ ] **3.4b Change log** ([manual 6.11](manual.md#611-change-log)): after 3.3 and 3.4 run
  `protonsync changes --days 1`. Expected: one line for the upload from 3.3 (side `Proton`) and
  one for the download from 3.4 (side `local`), nothing from the dry runs.

- [ ] **3.5 Create and resolve a conflict** ([manual 6.3](manual.md#63-conflicts))
  1. Change it locally: `echo "lokal" >> ~/protonsync-erprobung/sync/hallo.txt`
  2. In the browser, upload a different file named `hallo.txt` (other content) into the same
     folder and choose "Replace". Wait a minute.
  3. Sync and look:
     ```sh
     protonsync sync --apply
     protonsync conflicts
     ls ~/protonsync-erprobung/sync
     protonsync conflicts resolve erprobung NR keep-both
     protonsync conflicts
     ```
  Expected: `hallo.txt` holds the Proton version, next to it `hallo (Konflikt …).txt` holds your
  local version. `NR` is the number from the list; afterwards the list is empty.
  Error: one of the two versions is missing. That is a serious bug, please report it right away.

- [ ] **3.6 Trash and restore** ([manual 6.4](manual.md#64-trash))
  ```sh
  rm ~/protonsync-erprobung/sync/hallo.txt
  protonsync sync --apply
  protonsync trash
  protonsync trash restore erprobung NR
  protonsync sync --apply
  ```
  Expected: after deleting, the file is in the Proton trash, `trash` lists it, after restoring it
  is back in Proton and after the sync back locally.
  Then the other way round: delete the file in the browser, wait a minute, then
  `protonsync sync --apply`.
  Expected: the local copy is in `~/protonsync-erprobung/sync/.protonsync-trash` and can be brought
  back the same way with `protonsync trash restore erprobung NR`.
  `protonsync trash purge` only shows what would have expired; do **not** run it with `--apply`.

- [ ] **3.7 Automatic sync (optional, briefly without dry run)**
  ```sh
  sed -i 's/^dry_run = true/dry_run = false/' ~/.config/protonsync/config.toml
  systemctl --user restart protonsyncd
  protonsync status
  echo "automatisch" > ~/protonsync-erprobung/sync/auto.txt
  ```
  Expected: `status` shows dry-run mode as off; `auto.txt` appears in the browser after a few
  seconds, a file uploaded in the browser appears locally after about a minute. Then switch back:
  ```sh
  sed -i 's/^dry_run = false/dry_run = true/' ~/.config/protonsync/config.toml
  systemctl --user restart protonsyncd
  ```

- [ ] **3.8 FUSE drive**
  ([manual 4.7](manual.md#47-sections-mount-and-cache-fuse-drive-desktop-only),
  [6.5](manual.md#65-drive-always-keep-local-and-free-space))
  ```sh
  cat >> ~/.config/protonsync/config.toml <<EOF

  [[mount]]
  id = "laufwerk"
  mountpoint = "$HOME/protonsync-erprobung/laufwerk"
  remote = "protonsync-erprobung/laufwerk"
  keep_local = false

  [cache]
  evict_after_days = 30
  max_size_gb = 1
  EOF
  systemctl --user restart protonsyncd
  protonsync mounts
  ```
  Expected: the drive `laufwerk` is mounted. In the browser, upload a few files into
  `protonsync-erprobung/laufwerk` (e.g. a PDF you also have locally, and a video). Wait a minute,
  then (replace `DATEI.pdf` with the real name):
  ```sh
  D=~/protonsync-erprobung/laufwerk
  ls -l "$D"
  protonsync file-status "$D"/*
  cmp ~/Downloads/DATEI.pdf "$D/DATEI.pdf"
  protonsync file-status "$D/DATEI.pdf"
  protonsync pin "$D/DATEI.pdf"
  protonsync mounts
  protonsync unpin "$D/DATEI.pdf"
  protonsync free "$D"
  protonsync free "$D" --apply
  protonsync file-status "$D/DATEI.pdf"
  ```
  Expected: files appear at once with their full size as online only (placeholder); `cmp` prints
  nothing (same content); then available locally; `mounts` lists the pinned path; `free` first
  plans, then frees, and the file is a placeholder again but still visible. A video should start
  playing right away.
  Writing and deleting (takes effect immediately, no dry run):
  ```sh
  echo "neu" > "$D/neu.txt"
  rm "$D/neu.txt"
  protonsync trash
  protonsync trash restore mount:laufwerk NR
  ```
  Expected: `neu.txt` appears in the browser, is in the Proton trash after `rm` and is back after
  restoring. Error: `Transport endpoint is not connected` or "not mounted", see
  [manual 9.2](manual.md#92-common-problems).

- [ ] **3.9 Tray app on the desktop** ([manual 6.6](manual.md#66-tray-app))
  ```sh
  protonsync-tray &
  ```
  On GNOME, first turn on the extension "AppIndicator and KStatusNotifierItem Support".
  Check:
  - The icon appears; the tooltip shows state, account and dry-run mode.
  - A click opens the status window: pair `erprobung` and drive `laufwerk`; **Open folder** opens
    the file manager.
  - The menu (right click) shows **Start dry run**; a click does not change any files.
  - Conflict dialog: create a conflict as in 3.5 with a new file `notiz.txt` and run
    `protonsync sync --apply`. Expected: a notification, a yellow icon, then **Keep both** under
    **Conflicts**; afterwards a green icon and `protonsync conflicts` is empty.
  - Language: under **Settings** switch to English and back to German; all texts change. For the
    CLI: `LANG=en_US.UTF-8 protonsync status`.
  - Daemon settings (ADR 0021): under **Settings**, tick **Protection** for the pair `erprobung`.
    Expected: the message "Setting saved", `protonsync config show` shows `protect_remote = yes`,
    `config.toml` contains `protect_remote = true` and its comments are still there. Untick it
    again.
  - `systemctl --user restart protonsyncd`: the icon turns grey briefly and reconnects after a few
    seconds.
  - Tick **Start at login**, log out and back in: the tray app starts by itself.
  - If possible, look at it once under Wayland and once under X11.

  Error: no icon, the app crashes, or English texts although German is set. Then run
  `protonsync-tray --check` in a terminal and keep the output.

- [ ] **3.10 Dolphin or Nautilus** ([manual 6.9](manual.md#69-file-managers-dolphin-and-nautilus)).
  Install the files by hand (from `~/proton-drive-custom-client`):
  ```sh
  sudo pacman -S --needed libnotify
  install -Dm755 packaging/filemanager/protonsync-filemanager ~/.local/bin/protonsync-filemanager
  # Dolphin (KDE):
  install -Dm755 packaging/filemanager/dolphin/protonsync.desktop \
    ~/.local/share/kio/servicemenus/protonsync.desktop
  # Nautilus (GNOME):
  sudo pacman -S --needed python-nautilus
  install -Dm644 packaging/filemanager/nautilus/protonsync_nautilus.py \
    ~/.local/share/nautilus-python/extensions/protonsync_nautilus.py
  nautilus -q
  ```
  Expected: right-clicking a file in `~/protonsync-erprobung/laufwerk` shows the **protonsync**
  submenu with **Always keep local**, **On demand only**, **Free up space**; `protonsync
  file-status` shows the effect. Nautilus also shows emblems in the drive and in the sync folder
  (`F5` reloads). Error: no menu, or an error message as a notification.

- [ ] **3.11 A small upload job**
  ([manual 4.5](manual.md#45-section-job-upload-jobs-any-number-of-times))
  ```sh
  mkdir -p ~/protonsync-erprobung/job-quelle
  cd ~/protonsync-erprobung/job-quelle
  for n in 1 2 3; do echo "Backup $n" > backup-$n.tar; sleep 1; done
  cd -
  cat >> ~/.config/protonsync/config.toml <<EOF

  [[job]]
  id = "test-job"
  source = "$HOME/protonsync-erprobung/job-quelle"
  pattern = "*.tar"
  target = "protonsync-erprobung/job"
  schedule = "@daily"
  keep_last = 2
  settle_seconds = 5
  EOF
  systemctl --user restart protonsyncd
  protonsync jobs
  protonsync jobs run test-job
  ```
  Expected: `jobs` lists `test-job` with its next run; the dry run plans three uploads and names
  `backup-1.tar` as the file that retention (`keep_last = 2`) would move to the trash.
  Only when the plan is right:
  ```sh
  protonsync jobs run test-job --apply
  protonsync jobs history test-job
  protonsync trash
  protonsync trash restore job:test-job NR
  ```
  Expected: three files in the browser, then `backup-1.tar` in the trash under `job:test-job`, and
  back after restoring. Error: an upload without a plan first, or a deletion that does not show up
  in the trash.

- [ ] **3.12 Photos, read only at first**
  ([manual 4.8](manual.md#48-section-photos-photos-proton-photos),
  [6.8](manual.md#68-photos-and-duplicates))
  ```sh
  cat >> ~/.config/protonsync/config.toml <<EOF

  [photos]
  folder = "$HOME/protonsync-erprobung/fotos"
  direction = "download_only"
  EOF
  systemctl --user restart protonsyncd
  protonsync photos
  protonsync photos sync
  protonsync photos duplicates
  ```
  Expected: the number of photos, a plan with downloads only, and a list of possible duplicates
  (a suggestion only, nothing is deleted). If there is enough free space,
  `protonsync photos sync --apply`: the photos land under `fotos/YYYY/MM/`. With `download_only`
  nothing changes in Proton. Do **not** run `protonsync photos duplicates trash … --apply`.

### 4. Build and install the Arch package

- [ ] **4.1 Remove the `cargo` installation** (configuration and login stay,
  [manual 8](manual.md#8-updating-logging-out-uninstalling)). Quit the tray app from its menu
  first.
  ```sh
  systemctl --user disable --now protonsyncd
  rm ~/.config/systemd/user/protonsyncd.service
  systemctl --user daemon-reload
  cargo uninstall protonsync-daemon protonsync-cli
  cargo uninstall protonsync-gui-qt
  rm -f ~/.config/autostart/protonsync-tray.desktop ~/.local/bin/protonsync-filemanager \
    ~/.local/share/kio/servicemenus/protonsync.desktop \
    ~/.local/share/nautilus-python/extensions/protonsync_nautilus.py
  ```

- [ ] **4.2 Build the package** ([manual 3.5](manual.md#35-alternative-as-a-package-archcachyos))
  ```sh
  cd ~/proton-drive-custom-client/packaging/arch
  makepkg -si
  sudo pacman -S --needed libnotify python-nautilus
  pacman -Qi protonsync-git
  command -v protonsync
  ```
  `makepkg -si` builds the state of `main` on GitHub. To build your own checkout instead:
  `PROTONSYNC_GIT="file://$HOME/proton-drive-custom-client" makepkg -si`.
  Expected: package `protonsync-git` installed, `command -v` shows `/usr/bin/protonsync`.
  Error: `makepkg` stops (keep the last 50 lines of its output).

- [ ] **4.3 Start with the package**
  ```sh
  systemctl --user enable --now protonsyncd
  systemctl --user status protonsyncd
  protonsync status
  protonsync mounts
  ```
  Expected: running, still logged in, pair and drive as before. Start the tray app from the
  application menu ("protonsync") and tick **Start at login** again. The Dolphin or Nautilus menu
  (after `nautilus -q`) works as in 3.10.

- [ ] **4.4 Clean up when everything is done.** In the browser, delete `protonsync-erprobung` and
  `protonsync-live-tests`. Then remove the test sections from the configuration (or restore the
  backup from 1.6), restart the daemon and clean up locally:
  ```sh
  systemctl --user restart protonsyncd
  rm -r ~/protonsync-erprobung
  ```
  Log out with `protonsync logout` if you want.

### 5. Feedback when something goes wrong

Please send these details (to Claude in the project or as an issue in the repository):

- [ ] The step number, the exact command, what you expected and what you saw, the exit code
  (`echo $?` right after the command).
- [ ] Versions: `protonsync --version` and `protonsyncd --version` (with the package also
  `pacman -Qi protonsync-git`).
- [ ] The state as JSON:
  ```sh
  protonsync --json status > ~/protonsync-status.json
  ```
  For drive problems also `protonsync --json mounts`.
- [ ] The daemon log ([manual 9.1](manual.md#91-logs)): live with
  `journalctl --user -u protonsyncd -f`, as a file for the day:
  ```sh
  journalctl --user -u protonsyncd --since today --no-pager > ~/protonsync-log.txt
  ```
  For hard-to-catch errors, first set `RUST_LOG=debug` as in manual 9.1
  (`systemctl --user edit protonsyncd`), repeat the error, then remove it again.
- [ ] For the tray app: the output of `protonsync-tray --check` or of `protonsync-tray` in a
  terminal.

**Never send:** password, mailbox password, 2FA codes, values of the `PROTONSYNC_LIVE_*`
variables, the folder `~/.local/share/protonsync/secrets/`, keyring entries. The log contains no
passwords, but it does contain file names; read it before sending and replace names you do not
want to share.
