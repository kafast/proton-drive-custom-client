# ADR 0003: Asynchrone Ports ohne `async-trait`

- Status: angenommen
- Datum: 2026-10-16

## Kontext
Netzwerk- und Dateizugriffe laufen asynchron (Tokio, wie in den Proton-Crates). Rust erlaubt
`async fn` in Traits seit Version 1.75, verlangt aber für öffentliche Traits eine Aussage darüber,
ob die Futures zwischen Threads wandern dürfen (`Send`).

## Entscheidung
Port-Methoden werden als `fn name(&self, ...) -> impl Future<Output = Result<..>> + Send`
deklariert. Implementierungen dürfen einfach `async fn` schreiben. Das Crate `async-trait` wird
nicht verwendet.

## Folgen
- Keine zusätzliche Abhängigkeit und keine versteckten Heap-Allokationen pro Aufruf.
- Diese Traits sind nicht „dyn-kompatibel“: Use Cases bekommen ihre Ports als generische
  Parameter (`SyncEngine<R: RemoteStorage>`), vergleichbar mit Generics in Java, statt als
  `Box<dyn RemoteStorage>`. Der Zusammenbau passiert einmal im Daemon (Composition Root).
