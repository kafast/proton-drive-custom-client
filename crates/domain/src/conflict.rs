//! Entscheidung, was mit einer Datei beim Abgleich passiert, und Konflikte.
//!
//! Grundlage ist ein Drei-Wege-Vergleich: der zuletzt gemeinsam bekannte Stand
//! („Basis“), der lokale Stand und der entfernte Stand. `None` bedeutet jeweils
//! „existiert nicht (mehr)“.

use std::path::PathBuf;

use crate::node::Fingerprint;
use crate::sync_pair::SyncDirection;
use crate::time::Timestamp;

/// Was der Abgleich für eine Datei tun soll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncAction {
    /// Nichts tun, beide Seiten stimmen mit der Basis überein.
    Nothing,
    /// Beide Seiten haben sich gleich geändert; nur die Basis aktualisieren.
    RecordConverged,
    /// Lokale Datei hochladen.
    Upload,
    /// Entfernte Datei herunterladen.
    Download,
    /// Entfernte Datei in den Papierkorb legen (lokal gelöscht).
    TrashRemote,
    /// Lokale Datei in den Papierkorb legen (entfernt gelöscht).
    TrashLocal,
    /// Lokale Änderung wird wegen der Sync-Richtung nicht übertragen.
    IgnoreLocalChange,
    /// Entfernte Änderung wird wegen der Sync-Richtung nicht übernommen.
    IgnoreRemoteChange,
    /// Beide Seiten unterschiedlich geändert: den Nutzer fragen.
    Conflict,
}

/// Bestimmt die Aktion für eine Datei aus Basis, lokalem und entferntem Stand.
///
/// Regel: Bei Änderungen auf beiden Seiten wird **nie** automatisch eine Seite
/// überschrieben, auch nicht bei einseitiger Sync-Richtung. Daraus wird immer
/// ein [`SyncAction::Conflict`], außer beide Seiten sind identisch.
#[must_use]
pub fn decide(
    base: Option<&Fingerprint>,
    local: Option<&Fingerprint>,
    remote: Option<&Fingerprint>,
    direction: SyncDirection,
) -> SyncAction {
    let local_changed = local != base;
    let remote_changed = remote != base;

    match (local_changed, remote_changed) {
        (false, false) => SyncAction::Nothing,
        (true, false) if !direction.allows_upload() => SyncAction::IgnoreLocalChange,
        (true, false) if local.is_none() => SyncAction::TrashRemote,
        (true, false) => SyncAction::Upload,
        (false, true) if !direction.allows_download() => SyncAction::IgnoreRemoteChange,
        (false, true) if remote.is_none() => SyncAction::TrashLocal,
        (false, true) => SyncAction::Download,
        (true, true) if local == remote => SyncAction::RecordConverged,
        (true, true) => SyncAction::Conflict,
    }
}

/// Wie ein Konflikt aufgelöst werden soll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Lokale Version gewinnt, die entfernte geht in den Papierkorb.
    KeepLocal,
    /// Entfernte Version gewinnt, die lokale geht in den Papierkorb.
    KeepRemote,
    /// Beide behalten; die lokale wird als Konfliktkopie umbenannt.
    KeepBoth,
}

/// Ein offener Konflikt in der Warteschlange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// Pfad relativ zum Wurzelordner des Sync-Paares.
    pub relative_path: PathBuf,
    /// Lokaler Stand beim Erkennen (`None` = lokal gelöscht).
    pub local: Option<Fingerprint>,
    /// Entfernter Stand beim Erkennen (`None` = entfernt gelöscht).
    pub remote: Option<Fingerprint>,
    /// Wann der Konflikt erkannt wurde.
    pub detected_at: Timestamp,
}

/// Name der Konfliktkopie, z. B. `Bericht (Konflikt 1760000000).pdf`.
///
/// Die Endung bleibt erhalten, damit die Kopie mit demselben Programm öffnet.
#[must_use]
pub fn conflict_copy_name(original: &str, detected_at: Timestamp) -> String {
    let marker = format!("(Konflikt {})", detected_at.unix_seconds());
    match original.rsplit_once('.') {
        // Ein Punkt am Anfang (".bashrc") ist keine Endung.
        Some((stem, extension)) if !stem.is_empty() => format!("{stem} {marker}.{extension}"),
        _ => format!("{original} {marker}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn fp(size: u64) -> Fingerprint {
        Fingerprint {
            size,
            modified: Timestamp::from_unix_seconds(0),
            content_hash: None,
        }
    }

    const TWO_WAY: SyncDirection = SyncDirection::TwoWay;

    #[test]
    fn unchanged_file_needs_nothing() {
        let a = fp(1);
        assert_eq!(
            decide(Some(&a), Some(&a), Some(&a), TWO_WAY),
            SyncAction::Nothing
        );
    }

    #[test]
    fn local_edit_is_uploaded() {
        let (base, new) = (fp(1), fp(2));
        assert_eq!(
            decide(Some(&base), Some(&new), Some(&base), TWO_WAY),
            SyncAction::Upload
        );
    }

    #[test]
    fn new_local_file_is_uploaded() {
        assert_eq!(
            decide(None, Some(&fp(1)), None, TWO_WAY),
            SyncAction::Upload
        );
    }

    #[test]
    fn remote_edit_is_downloaded() {
        let (base, new) = (fp(1), fp(2));
        assert_eq!(
            decide(Some(&base), Some(&base), Some(&new), TWO_WAY),
            SyncAction::Download
        );
    }

    #[test]
    fn deletions_go_to_trash() {
        let base = fp(1);
        assert_eq!(
            decide(Some(&base), None, Some(&base), TWO_WAY),
            SyncAction::TrashRemote
        );
        assert_eq!(
            decide(Some(&base), Some(&base), None, TWO_WAY),
            SyncAction::TrashLocal
        );
    }

    #[test]
    fn different_edits_on_both_sides_are_a_conflict() {
        let base = fp(1);
        assert_eq!(
            decide(Some(&base), Some(&fp(2)), Some(&fp(3)), TWO_WAY),
            SyncAction::Conflict
        );
    }

    #[test]
    fn edit_versus_delete_is_a_conflict() {
        let base = fp(1);
        assert_eq!(
            decide(Some(&base), Some(&fp(2)), None, TWO_WAY),
            SyncAction::Conflict
        );
    }

    #[test]
    fn identical_edits_converge() {
        let base = fp(1);
        let same = fp(2);
        assert_eq!(
            decide(Some(&base), Some(&same), Some(&same), TWO_WAY),
            SyncAction::RecordConverged
        );
    }

    #[test]
    fn one_way_directions_ignore_the_other_side() {
        let (base, new) = (fp(1), fp(2));
        assert_eq!(
            decide(
                Some(&base),
                Some(&new),
                Some(&base),
                SyncDirection::DownloadOnly
            ),
            SyncAction::IgnoreLocalChange
        );
        assert_eq!(
            decide(
                Some(&base),
                Some(&base),
                Some(&new),
                SyncDirection::UploadOnly
            ),
            SyncAction::IgnoreRemoteChange
        );
    }

    #[test]
    fn conflict_copy_keeps_extension() {
        let at = Timestamp::from_unix_seconds(42);
        assert_eq!(
            conflict_copy_name("Bericht.pdf", at),
            "Bericht (Konflikt 42).pdf"
        );
        assert_eq!(conflict_copy_name("Makefile", at), "Makefile (Konflikt 42)");
        assert_eq!(conflict_copy_name(".bashrc", at), ".bashrc (Konflikt 42)");
    }

    fn any_state() -> impl Strategy<Value = Option<Fingerprint>> {
        proptest::option::of((0u64..4).prop_map(fp))
    }

    fn any_direction() -> impl Strategy<Value = SyncDirection> {
        prop_oneof![
            Just(SyncDirection::TwoWay),
            Just(SyncDirection::DownloadOnly),
            Just(SyncDirection::UploadOnly),
        ]
    }

    proptest! {
        /// Eine Seite, die sich geändert hat, wird nie stillschweigend überschrieben.
        #[test]
        fn never_overwrites_a_changed_side(
            base in any_state(), local in any_state(), remote in any_state(),
            direction in any_direction(),
        ) {
            let action = decide(base.as_ref(), local.as_ref(), remote.as_ref(), direction);
            if local != base {
                prop_assert!(!matches!(action, SyncAction::Download | SyncAction::TrashLocal));
            }
            if remote != base {
                prop_assert!(!matches!(action, SyncAction::Upload | SyncAction::TrashRemote));
            }
        }

        /// Die Sync-Richtung wird immer eingehalten.
        #[test]
        fn respects_direction(
            base in any_state(), local in any_state(), remote in any_state(),
            direction in any_direction(),
        ) {
            let action = decide(base.as_ref(), local.as_ref(), remote.as_ref(), direction);
            if !direction.allows_upload() {
                prop_assert!(!matches!(action, SyncAction::Upload | SyncAction::TrashRemote));
            }
            if !direction.allows_download() {
                prop_assert!(!matches!(action, SyncAction::Download | SyncAction::TrashLocal));
            }
        }

        /// Gleiche Stände auf beiden Seiten sind nie ein Konflikt.
        #[test]
        fn equal_sides_are_never_a_conflict(
            base in any_state(), both in any_state(), direction in any_direction(),
        ) {
            let action = decide(base.as_ref(), both.as_ref(), both.as_ref(), direction);
            prop_assert_ne!(action, SyncAction::Conflict);
        }
    }
}
