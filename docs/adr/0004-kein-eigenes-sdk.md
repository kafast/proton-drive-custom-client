# ADR 0004: Vorerst kein eigenes Proton-SDK

- Status: angenommen
- Datum: 2026-10-16

## Kontext
Der Auftraggeber hat gefragt, ob statt der Community-Crates (ADR 0002) eine eigene API-Anbindung,
also ein eigenes SDK, gebaut werden soll. Die Proton-API ist für Drittanbieter nicht dokumentiert;
ein eigenes SDK müsste aus dem offiziellen SDK-Code (MIT) nachgebaut werden: SRP-Login, 2FA,
PGP-Schlüsselkette (Konto → Adresse → Freigabe → Knoten), blockweise Verschlüsselung mit
Signaturen, Revisionen, Events, Photos, Schutzmechanismen gegen Missbrauch. Proton stellt zudem
Ende 2026/Anfang 2027 auf ein neues Kryptografie-Modell um.

## Entscheidung
Kein eigenes SDK zum jetzigen Zeitpunkt. Stattdessen:
1. Community-Crates `proton-sdk`/`proton-drive-rs` exakt gepinnt, nur in `adapter-proton`.
2. Lücken werden im Adapter ergänzt oder dem Community-Projekt gemeldet bzw. beigesteuert.
3. Wird das Community-Projekt aufgegeben: Fork und eigene Pflege.
4. Neubewertung, sobald Protons neues Kryptografie-Modell und das Drittanbieter-SDK stabil sind.

## Folgen
- Kein eigener Kryptografie-Code; geringeres Risiko, die Zero-Knowledge-Verschlüsselung zu schwächen.
- Abhängigkeit von einem Community-Projekt; abgefedert durch den Port `RemoteStorage` und die
  Contract-Tests: Ein eigenes SDK wäre später nur ein weiterer Adapter.
