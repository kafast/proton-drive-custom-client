//! Bilingual descriptions of the entries in the change log, shared by the
//! CLI and the daily digest of the daemon.

use protonsync_api::{ChangeInfo, ChangeKindInfo, ChangedSideInfo};

use crate::Lang;

/// What happened to the entry, as one short sentence fragment,
/// e.g. "neu in Proton" / "new in Proton".
#[must_use]
pub fn describe(lang: Lang, change: &ChangeInfo) -> &'static str {
    match change.side {
        ChangedSideInfo::Remote => in_proton(lang, change.kind, change.is_folder),
        ChangedSideInfo::Local => from_proton(lang, change.kind, change.is_folder),
    }
}

/// Changes made in Proton (they came from the local folder).
fn in_proton(lang: Lang, kind: ChangeKindInfo, is_folder: bool) -> &'static str {
    match kind {
        ChangeKindInfo::Created if is_folder => {
            lang.pick("Ordner in Proton angelegt", "folder created in Proton")
        }
        ChangeKindInfo::Created => {
            lang.pick("neu in Proton hochgeladen", "uploaded to Proton (new)")
        }
        ChangeKindInfo::Modified => lang.pick(
            "neue Version in Proton hochgeladen",
            "new version uploaded to Proton",
        ),
        ChangeKindInfo::Trashed => lang.pick(
            "in Proton in den Papierkorb verschoben",
            "moved to the trash in Proton",
        ),
        ChangeKindInfo::Archived => lang.pick(
            "in Proton ins Archiv verschoben",
            "moved to the archive in Proton",
        ),
        ChangeKindInfo::ArchivedAndReplaced => lang.pick(
            "in Proton archiviert und durch die lokale Version ersetzt",
            "archived in Proton and replaced by the local version",
        ),
        ChangeKindInfo::KeptCopy => lang.pick("in Proton behalten", "kept in Proton"),
        ChangeKindInfo::Conflict => lang.pick(
            "Konflikt: lokale Version in Proton wieder angelegt",
            "conflict: local version recreated in Proton",
        ),
    }
}

/// Changes from Proton applied to the local folder.
fn from_proton(lang: Lang, kind: ChangeKindInfo, is_folder: bool) -> &'static str {
    match kind {
        ChangeKindInfo::Created if is_folder => lang.pick(
            "Ordner aus Proton lokal angelegt",
            "folder from Proton created locally",
        ),
        ChangeKindInfo::Created => lang.pick(
            "neu in Proton, heruntergeladen",
            "new in Proton, downloaded",
        ),
        ChangeKindInfo::Modified => lang.pick(
            "in Proton geändert, neue Version heruntergeladen",
            "changed in Proton, new version downloaded",
        ),
        ChangeKindInfo::Trashed => lang.pick(
            "in Proton gelöscht, lokal in den Papierkorb verschoben",
            "deleted in Proton, moved to the local trash",
        ),
        ChangeKindInfo::Archived => lang.pick(
            "in Proton gelöscht, lokal ins Archiv verschoben",
            "deleted in Proton, moved to the local archive",
        ),
        ChangeKindInfo::ArchivedAndReplaced => lang.pick(
            "in Proton geändert, alte Version lokal archiviert",
            "changed in Proton, old version archived locally",
        ),
        ChangeKindInfo::KeptCopy => lang.pick(
            "in Proton gelöscht, lokale Kopie behalten",
            "deleted in Proton, local copy kept",
        ),
        ChangeKindInfo::Conflict => lang.pick(
            "Konflikt: beide Versionen behalten",
            "conflict: both versions kept",
        ),
    }
}

/// Short name of the changed side for tables: "Proton" or "lokal"/"local".
#[must_use]
pub fn side(lang: Lang, side: ChangedSideInfo) -> &'static str {
    match side {
        ChangedSideInfo::Remote => "Proton",
        ChangedSideInfo::Local => lang.pick("lokal", "local"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [ChangeKindInfo; 7] = [
        ChangeKindInfo::Created,
        ChangeKindInfo::Modified,
        ChangeKindInfo::Trashed,
        ChangeKindInfo::Archived,
        ChangeKindInfo::ArchivedAndReplaced,
        ChangeKindInfo::KeptCopy,
        ChangeKindInfo::Conflict,
    ];

    fn change(side: ChangedSideInfo, kind: ChangeKindInfo, is_folder: bool) -> ChangeInfo {
        ChangeInfo {
            pair: "main".into(),
            at: 0,
            path: "a".into(),
            side,
            kind,
            is_folder,
        }
    }

    #[test]
    fn every_change_has_its_own_text_in_both_languages() {
        for lang in [Lang::De, Lang::En] {
            let mut seen = std::collections::HashSet::new();
            for side in [ChangedSideInfo::Local, ChangedSideInfo::Remote] {
                for kind in KINDS {
                    assert!(seen.insert(describe(lang, &change(side, kind, false))));
                }
                assert!(seen.insert(describe(lang, &change(side, ChangeKindInfo::Created, true))));
            }
        }
        assert_ne!(
            describe(
                Lang::De,
                &change(ChangedSideInfo::Local, ChangeKindInfo::Created, false)
            ),
            describe(
                Lang::En,
                &change(ChangedSideInfo::Local, ChangeKindInfo::Created, false)
            )
        );
    }
}
