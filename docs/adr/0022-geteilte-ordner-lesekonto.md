# ADR 0022: Sync-Paar auf einen geteilten Ordner (Lese-Konto)

- Status: angenommen
- Datum: 2026-10-30

## Kontext
Der Auftraggeber möchte den Client nutzen, ohne sich darauf verlassen zu müssen, dass der noch wenig
erprobte Code nie etwas in Proton ändert. Proton Drive bietet keine API-Keys oder Tokens mit
Leserechten (geprüft 2026-10-30; Tokens gibt es nur für die Proton Pass CLI). Proton kann aber
Ordner per E-Mail-Einladung mit einem anderen Proton-Konto teilen, mit der Rolle „Betrachter“
(nur lesen) oder „Bearbeiter“. Diese Rechte setzt Proton auf dem Server durch.

## Entscheidung
- **Lese-Konto:** Der Nutzer legt ein zweites Proton-Konto an, teilt im Hauptkonto die gewünschten
  Ordner als „Betrachter“ mit ihm, nimmt die Einladung im Web an und meldet protonsync mit dem
  Lese-Konto an. Das Passwort des Hauptkontos liegt dann nirgends bei protonsync.
- **Konfiguration:** `shared = true` in einem `[[pair]]`. `remote` ist dann
  `<Name des geteilten Ordners>[/Unterordner]` unter „Mit mir geteilt“.
- **Nur herunterladen:** Mit `shared = true` sind nur `download_only` und `backup_only` erlaubt; die
  Prüfung steckt im Konfigurations-Parser und gilt damit auch für Änderungen über Tray, Web und CLI
  (ADR 0021). Angelegt wird nichts: Fehlt der Ordner, meldet die Verbindung den Grund und versucht es
  beim nächsten Lauf erneut.
- **Port:** `Account::open_shared_remote(path)` neben `open_remote`. Die Anwendungsschicht bekommt
  `find_folder` (sucht wie `ensure_folder`, legt aber nichts an).
- **Proton-Adapter:** `ProtonRemote::shared_with_me(name)` sucht den Ordner über
  `enumerate_shared_with_me_node_uids` (Alben und Fotos sind dort ausgefiltert). Gibt es den Namen
  nicht, mehrfach oder nur als Datei, wird mit klarer Meldung abgebrochen. Danach arbeitet
  `ProtonRemote` wie gewohnt mit diesem Ordner als Wurzel; die Ereignisse kommen aus dem Volume des
  Besitzers (`root.volume_id`).
- Laufwerke (`[[mount]]`) und Fotos bleiben vorerst bei „Meine Dateien“ bzw. Proton Photos.

## Folgen
- Selbst ein Fehler in protonsync oder ein kompromittierter Server kann über das Lese-Konto nichts
  in Proton ändern oder löschen. Zusammen mit `backup_only` (ADR 0019) entsteht ein lokales Backup,
  das Löschungen und Verschlüsselung in Proton übersteht, weil alte Versionen ins `_archiv` wandern.
- **Noch nicht mit echtem Konto geprüft:** ob Proton „Betrachter“ wirklich auch Löschen verweigert,
  und ob die Ereignisse des fremden Volumes für das Lese-Konto lesbar sind. Beides steht in
  `docs/erprobung.md`. Falls die Ereignisse nicht lesbar sind, muss als Rückfall der periodische
  vollständige Abgleich genügen (eigene Änderung).
- Status, CLI und Oberflächen zeigen `remote` unverändert an; dass es ein geteilter Ordner ist,
  steht nur in der Konfiguration.

---

## English

**ADR 0022: Sync pair on a shared folder (reader account)**

- Status: accepted
- Date: 2026-10-30

### Context
The client wants to use protonsync without relying on the still little-tested code never changing
anything in Proton. Proton Drive offers no API keys or read-only tokens (checked 2026-10-30; tokens
exist only for the Proton Pass CLI). Proton can, however, share folders by email invitation with
another Proton account, with the role "Viewer" (read only) or "Editor". Proton enforces these
rights on the server.

### Decision
- **Reader account:** the user creates a second Proton account, shares the wanted folders with it
  as "Viewer" from the main account, accepts the invitation on the web and logs protonsync in with
  the reader account. The main account's password is then nowhere near protonsync.
- **Configuration:** `shared = true` in a `[[pair]]`. `remote` is then
  `<name of the shared folder>[/subfolder]` in "Shared with me".
- **Download only:** with `shared = true` only `download_only` and `backup_only` are allowed; the
  check lives in the configuration parser and therefore also applies to changes made through tray,
  web and CLI (ADR 0021). Nothing is created: if the folder is missing, the connection reports the
  reason and tries again at the next run.
- **Port:** `Account::open_shared_remote(path)` next to `open_remote`. The application layer gets
  `find_folder` (searches like `ensure_folder` but creates nothing).
- **Proton adapter:** `ProtonRemote::shared_with_me(name)` finds the folder via
  `enumerate_shared_with_me_node_uids` (albums and photos are filtered out there). If the name is
  missing, ambiguous or only a file, it stops with a clear message. After that `ProtonRemote` works
  as usual with this folder as its root; events come from the owner's volume (`root.volume_id`).
- Drives (`[[mount]]`) and photos stay with "My files" and Proton Photos for now.

### Consequences
- Even a bug in protonsync or a compromised server cannot change or delete anything in Proton
  through the reader account. Together with `backup_only` (ADR 0019) this gives a local backup that
  survives deletions and encryption in Proton, because old versions move to `_archiv`.
- **Not yet checked with a real account:** whether Proton's "Viewer" really refuses deleting too,
  and whether the reader account can read the events of the foreign volume. Both are listed in
  `docs/erprobung.md`. Should the events not be readable, the periodic full sync has to serve as a
  fallback (a separate change).
- Status, CLI and front ends show `remote` unchanged; that it is a shared folder is only visible in
  the configuration.
