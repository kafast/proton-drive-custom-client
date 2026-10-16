# ADR 0007: Entfernter Baum im Zwischenspeicher, nachgeführt über Proton-Ereignisse

- Status: angenommen
- Datum: 2026-10-16

## Kontext
In Phase 1 las jeder Lauf den ganzen entfernten Baum (ein `list_children` je Ordner, dazu das
Entschlüsseln aller Namen). Jede lokale Änderung und jedes Proton-Ereignis löste so einen
vollständigen Durchlauf aus. Das widerspricht der Vorgabe von Proton (Ereignisse statt
rekursivem Durchlaufen) und wird bei großen Bäumen langsam.

## Entscheidung
- `RemoteCache` (Anwendungsschicht, `sync::remote_index`) hält den entfernten Baum je Sync-Paar
  im Speicher, samt Ereignis-Cursor.
- Vor jedem Lauf holt er nur die Ereignisse seit dem Cursor. Je Knoten zählt das erste Ereignis
  für die Reihenfolge und das letzte für den Zustand; für geänderte Knoten wird der **aktuelle**
  Stand geladen (`node`). Neue oder verschobene Ordner werden samt Inhalt gelesen; bei
  unbekanntem Elternordner wird die Elternkette hochgegangen (innen: Unterbaum lesen;
  außen: merken und ignorieren).
- Vollständig neu gelesen wird bei: leerem Zwischenspeicher (Start, neue Anmeldung), Lücke im
  Ereignisstrom (`RemoteEvent::Gap`), mehr als 1000 Ereignissen, Papierkorb der Paar-Wurzel,
  unklarer Lage, dem Sicherheitsnetz (`full_sync_minutes`) und manuellem `protonsync sync`.
- Eigene Änderungen (Upload, Ordner, Papierkorb) trägt die Engine sofort ein, ohne auf ihr
  Ereignis zu warten.
- Sicherheitsregel: Stammt der Baum aus Ereignissen, wird vor jedem lokalen Löschen geprüft,
  ob der Knoten in Proton wirklich fehlt. Ist er noch da, wird der Schritt nicht ausgeführt,
  der Zwischenspeicher verworfen und sofort ein vollständiger Lauf angestoßen.

## Folgen
- Ein Lauf ohne entfernte Änderungen kostet bei Proton einen einzigen Aufruf.
- Der Zwischenspeicher lebt nur im Speicher; nach einem Neustart wird einmal vollständig gelesen.
  Eine dauerhafte Ablage in SQLite ist möglich, falls Neustarts bei sehr großen Bäumen stören.
- Ein Property-Test (`application/tests/remote_cache.rs`) prüft, dass der Zwischenspeicher nach
  beliebigen Folgen von Anlegen, Umbenennen, Verschieben (auch hinein und hinaus), Papierkorb
  und Wiederherstellen genau dem vollständigen Lesen entspricht.
- Der lokale Ordner wird weiterhin bei jedem Lauf gescannt (nur Metadaten, ohne Prüfsummen).

---

## English

# ADR 0007: Remote tree cached in memory, kept current through Proton events

- Status: accepted
- Date: 2026-10-16

### Context
In Phase 1 every run read the whole remote tree (one `list_children` call per folder, plus
decrypting every name). Every local change and every Proton event therefore triggered a full
traversal. That contradicts Proton's guidance (events instead of recursive traversal) and gets
slow for large trees.

### Decision
- `RemoteCache` (application layer, `sync::remote_index`) keeps the remote tree of each sync pair
  in memory, together with the event cursor.
- Before each run it fetches only the events since the cursor. Per node, the first event decides
  the processing order and the last one decides the state; for changed nodes the **current**
  state is loaded (`node`). New or moved folders are read including their content; if the parent
  folder is unknown, the parent chain is walked up (inside: read the subtree; outside: remember
  and ignore).
- The tree is read completely when: the cache is empty (start, new login), there is a gap in the
  event stream (`RemoteEvent::Gap`), there are more than 1000 events, the pair root was trashed,
  the location is unclear, on the safety-net interval (`full_sync_minutes`) and on a manual
  `protonsync sync`.
- The engine records its own changes (uploads, folders, trash) immediately instead of waiting for
  their events.
- Safety rule: if the tree comes from events, every local deletion first checks whether the node
  is really gone in Proton. If it still exists, the step is not executed, the cache is discarded
  and a full run is started right away.

### Consequences
- A run without remote changes costs a single call to Proton.
- The cache lives only in memory; after a restart the tree is read once in full. Persisting it
  in SQLite is possible if restarts become a problem for very large trees.
- A property test (`application/tests/remote_cache.rs`) checks that after arbitrary sequences of
  create, rename, move (also into and out of the pair), trash and restore the cache equals a full
  read.
- The local folder is still scanned on every run (metadata only, no checksums).
