//! Entscheidung, was mit einem Eintrag beim Abgleich passiert, und Konflikte.
//!
//! Grundlage ist ein Drei-Wege-Vergleich: Für jede Seite (lokal, entfernt) wird
//! der aktuelle Stand mit dem zuletzt gemeinsam bekannten Stand („Basis“)
//! verglichen. Die Seiten haben unterschiedliche Merkmale (lokal: Größe und
//! Änderungszeit, entfernt: Revisionskennung), deshalb wird jede Seite für sich
//! zu einer [`Change`] verdichtet.

use crate::path::RelativePath;
use crate::sync_pair::SyncDirection;
use crate::time::Timestamp;

/// Wie sich eine Seite seit der Basis verändert hat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Unverändert (oder auf beiden Zeitpunkten nicht vorhanden).
    Unchanged,
    /// Neu angelegt oder inhaltlich geändert.
    Modified,
    /// Seit der Basis gelöscht.
    Deleted,
}

impl Change {
    /// Vergleicht Basis und aktuellen Stand einer Seite. `None` = nicht vorhanden.
    #[must_use]
    pub fn detect<T: PartialEq>(base: Option<&T>, current: Option<&T>) -> Self {
        match (base, current) {
            (None, None) => Self::Unchanged,
            (Some(_), None) => Self::Deleted,
            (Some(base), Some(current)) if base == current => Self::Unchanged,
            (_, Some(_)) => Self::Modified,
        }
    }
}

/// Was der Abgleich für einen Eintrag tun soll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncAction {
    /// Nichts tun, beide Seiten stimmen mit der Basis überein.
    Nothing,
    /// Beide Seiten haben sich gleich geändert; nur die Basis aktualisieren.
    RecordConverged,
    /// Lokalen Stand hochladen.
    Upload,
    /// Entfernten Stand herunterladen.
    Download,
    /// Entfernten Eintrag in den Papierkorb legen (lokal gelöscht).
    TrashRemote,
    /// Lokalen Eintrag in den Papierkorb legen (entfernt gelöscht).
    TrashLocal,
    /// Lokale Änderung wird wegen der Sync-Richtung nicht übertragen.
    IgnoreLocalChange,
    /// Entfernte Änderung wird wegen der Sync-Richtung nicht übernommen.
    IgnoreRemoteChange,
    /// Beide Seiten unterschiedlich geändert: beide Versionen behalten und nachfragen.
    Conflict,
}

/// Bestimmt die Aktion für einen Eintrag.
///
/// `contents_identical` sagt, ob beide Seiten nach der Änderung denselben Inhalt
/// haben (nur relevant, wenn beide geändert wurden; im Zweifel `false`).
///
/// Regel: Bei Änderungen auf beiden Seiten wird **nie** automatisch eine Seite
/// überschrieben, auch nicht bei einseitiger Sync-Richtung. Daraus wird immer
/// ein [`SyncAction::Conflict`], außer beide Seiten sind gleich.
#[must_use]
pub fn decide(
    local: Change,
    remote: Change,
    contents_identical: bool,
    direction: SyncDirection,
) -> SyncAction {
    use Change::{Deleted, Modified, Unchanged};
    match (local, remote) {
        (Unchanged, Unchanged) => SyncAction::Nothing,
        (Modified | Deleted, Unchanged) if !direction.allows_upload() => {
            SyncAction::IgnoreLocalChange
        }
        (Modified, Unchanged) => SyncAction::Upload,
        (Deleted, Unchanged) => SyncAction::TrashRemote,
        (Unchanged, Modified | Deleted) if !direction.allows_download() => {
            SyncAction::IgnoreRemoteChange
        }
        (Unchanged, Modified) => SyncAction::Download,
        (Unchanged, Deleted) => SyncAction::TrashLocal,
        (Deleted, Deleted) => SyncAction::RecordConverged,
        (Modified, Modified) if contents_identical => SyncAction::RecordConverged,
        (Modified, Modified | Deleted) | (Deleted, Modified) => SyncAction::Conflict,
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

/// Welche Änderungen zum Konflikt geführt haben.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// Beide Seiten haben den Inhalt unterschiedlich geändert.
    BothModified,
    /// Lokal geändert, entfernt gelöscht. Die lokale Version bleibt erhalten.
    LocalModifiedRemoteDeleted,
    /// Lokal gelöscht, entfernt geändert. Die entfernte Version bleibt erhalten.
    LocalDeletedRemoteModified,
}

impl ConflictKind {
    /// Leitet die Art aus den Änderungen beider Seiten ab; `None`, wenn das kein Konflikt ist.
    #[must_use]
    pub const fn from_changes(local: Change, remote: Change) -> Option<Self> {
        match (local, remote) {
            (Change::Modified, Change::Modified) => Some(Self::BothModified),
            (Change::Modified, Change::Deleted) => Some(Self::LocalModifiedRemoteDeleted),
            (Change::Deleted, Change::Modified) => Some(Self::LocalDeletedRemoteModified),
            _ => None,
        }
    }
}

/// Ein offener Konflikt in der Warteschlange.
///
/// Beim Erkennen werden immer beide Versionen behalten: Bei
/// [`ConflictKind::BothModified`] wird die lokale Version als Konfliktkopie
/// (`conflict_copy`) daneben gelegt, die entfernte liegt unter dem Originalnamen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// Pfad relativ zum Wurzelordner des Sync-Paares.
    pub relative_path: RelativePath,
    /// Art des Konflikts.
    pub kind: ConflictKind,
    /// Pfad der Konfliktkopie relativ zum Wurzelordner, falls eine angelegt wurde.
    pub conflict_copy: Option<RelativePath>,
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
    use Change::{Deleted, Modified, Unchanged};
    use proptest::prelude::*;

    const TWO_WAY: SyncDirection = SyncDirection::TwoWay;

    #[test]
    fn detect_compares_base_and_current() {
        assert_eq!(Change::detect::<u8>(None, None), Unchanged);
        assert_eq!(Change::detect(Some(&1), Some(&1)), Unchanged);
        assert_eq!(Change::detect(Some(&1), Some(&2)), Modified);
        assert_eq!(Change::detect(None, Some(&1)), Modified);
        assert_eq!(Change::detect(Some(&1), None), Deleted);
    }

    #[test]
    fn unchanged_needs_nothing() {
        assert_eq!(
            decide(Unchanged, Unchanged, false, TWO_WAY),
            SyncAction::Nothing
        );
    }

    #[test]
    fn one_sided_changes_are_transferred() {
        assert_eq!(
            decide(Modified, Unchanged, false, TWO_WAY),
            SyncAction::Upload
        );
        assert_eq!(
            decide(Unchanged, Modified, false, TWO_WAY),
            SyncAction::Download
        );
    }

    #[test]
    fn deletions_go_to_trash() {
        assert_eq!(
            decide(Deleted, Unchanged, false, TWO_WAY),
            SyncAction::TrashRemote
        );
        assert_eq!(
            decide(Unchanged, Deleted, false, TWO_WAY),
            SyncAction::TrashLocal
        );
    }

    #[test]
    fn different_edits_on_both_sides_are_a_conflict() {
        assert_eq!(
            decide(Modified, Modified, false, TWO_WAY),
            SyncAction::Conflict
        );
    }

    #[test]
    fn edit_versus_delete_is_a_conflict() {
        assert_eq!(
            decide(Modified, Deleted, false, TWO_WAY),
            SyncAction::Conflict
        );
        assert_eq!(
            decide(Deleted, Modified, false, TWO_WAY),
            SyncAction::Conflict
        );
    }

    #[test]
    fn identical_results_converge() {
        assert_eq!(
            decide(Modified, Modified, true, TWO_WAY),
            SyncAction::RecordConverged
        );
        assert_eq!(
            decide(Deleted, Deleted, false, TWO_WAY),
            SyncAction::RecordConverged
        );
    }

    #[test]
    fn one_way_directions_ignore_the_other_side() {
        let download_only = SyncDirection::DownloadOnly;
        let upload_only = SyncDirection::UploadOnly;
        assert_eq!(
            decide(Modified, Unchanged, false, download_only),
            SyncAction::IgnoreLocalChange
        );
        assert_eq!(
            decide(Deleted, Unchanged, false, download_only),
            SyncAction::IgnoreLocalChange
        );
        assert_eq!(
            decide(Unchanged, Modified, false, upload_only),
            SyncAction::IgnoreRemoteChange
        );
        assert_eq!(
            decide(Unchanged, Deleted, false, upload_only),
            SyncAction::IgnoreRemoteChange
        );
    }

    #[test]
    fn conflict_kind_matches_changes() {
        assert_eq!(
            ConflictKind::from_changes(Modified, Modified),
            Some(ConflictKind::BothModified)
        );
        assert_eq!(
            ConflictKind::from_changes(Modified, Deleted),
            Some(ConflictKind::LocalModifiedRemoteDeleted)
        );
        assert_eq!(
            ConflictKind::from_changes(Deleted, Modified),
            Some(ConflictKind::LocalDeletedRemoteModified)
        );
        assert_eq!(ConflictKind::from_changes(Unchanged, Modified), None);
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

    fn any_change() -> impl Strategy<Value = Change> {
        prop_oneof![Just(Unchanged), Just(Modified), Just(Deleted)]
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
            local in any_change(), remote in any_change(), identical: bool,
            direction in any_direction(),
        ) {
            let action = decide(local, remote, identical, direction);
            if local != Unchanged {
                prop_assert!(!matches!(action, SyncAction::Download | SyncAction::TrashLocal));
            }
            if remote != Unchanged {
                prop_assert!(!matches!(action, SyncAction::Upload | SyncAction::TrashRemote));
            }
        }

        /// Die Sync-Richtung wird immer eingehalten.
        #[test]
        fn respects_direction(
            local in any_change(), remote in any_change(), identical: bool,
            direction in any_direction(),
        ) {
            let action = decide(local, remote, identical, direction);
            if !direction.allows_upload() {
                prop_assert!(!matches!(action, SyncAction::Upload | SyncAction::TrashRemote));
            }
            if !direction.allows_download() {
                prop_assert!(!matches!(action, SyncAction::Download | SyncAction::TrashLocal));
            }
        }

        /// Nur wenn beide Seiten geändert wurden, kann ein Konflikt entstehen.
        #[test]
        fn conflicts_need_changes_on_both_sides(
            local in any_change(), remote in any_change(), identical: bool,
            direction in any_direction(),
        ) {
            if decide(local, remote, identical, direction) == SyncAction::Conflict {
                prop_assert!(local != Unchanged && remote != Unchanged);
            }
        }
    }
}
