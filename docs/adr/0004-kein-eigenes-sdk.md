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

---

## English

**ADR 0004: No custom Proton SDK for now**

- Status: accepted
- Date: 2026-10-16

### Context
The client asked whether a custom API integration, that is, a custom SDK, should be built instead of
using the community crates (ADR 0002). The Proton API is not documented for third parties;
a custom SDK would have to be rebuilt from the official SDK code (MIT): SRP login, 2FA,
PGP key chain (account → address → share → node), block-wise encryption with
signatures, revisions, events, Photos, abuse protection mechanisms. In addition, Proton is switching
to a new cryptography model at the end of 2026/early 2027.

### Decision
No custom SDK at this time. Instead:
1. Community crates `proton-sdk`/`proton-drive-rs` pinned exactly, only in `adapter-proton`.
2. Gaps are filled in the adapter or reported or contributed to the community project.
3. If the community project is abandoned: fork it and maintain it ourselves.
4. Re-evaluate once Proton's new cryptography model and the third-party SDK are stable.

### Consequences
- No custom cryptography code; lower risk of weakening the zero-knowledge encryption.
- Dependency on a community project; mitigated by the port `RemoteStorage` and the
  contract tests: a custom SDK would later just be another adapter.
