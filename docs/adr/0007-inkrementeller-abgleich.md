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
