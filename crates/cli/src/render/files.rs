//! Text output for `file-status`.

use std::fmt::Write;

use protonsync_api::{PathState, PathStatusInfo};

use crate::i18n::Lang;

fn state_label(lang: Lang, state: PathState) -> &'static str {
    match state {
        PathState::Outside => lang.pick("nicht verwaltet", "not managed"),
        PathState::Folder => lang.pick("Ordner", "folder"),
        PathState::OnlineOnly => lang.pick("nur online (Platzhalter)", "online only (placeholder)"),
        PathState::Partial => lang.pick("teilweise geladen", "partly downloaded"),
        PathState::Local => lang.pick("lokal verfügbar", "available locally"),
        PathState::PendingUpload => lang.pick("wartet auf Upload", "waiting for upload"),
        PathState::Synced => lang.pick("synchron", "in sync"),
        PathState::Pending => lang.pick("wird abgeglichen", "pending sync"),
        PathState::Conflict => lang.pick("Konflikt", "conflict"),
    }
}

/// One path per line with its state.
#[must_use]
pub fn path_status(lang: Lang, entries: &[PathStatusInfo]) -> String {
    let mut out = String::new();
    for entry in entries {
        let _ = write!(out, "{}: {}", entry.path, state_label(lang, entry.state));
        if entry.pinned {
            let _ = write!(out, ", {}", lang.pick("immer lokal", "always local"));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_status_snapshots() {
        let entries = [
            (PathState::OnlineOnly, true),
            (PathState::Synced, false),
            (PathState::Conflict, false),
            (PathState::Outside, false),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (state, pinned))| PathStatusInfo {
            path: format!("/home/k/file{index}"),
            state,
            in_drive: pinned,
            pinned,
        })
        .collect::<Vec<_>>();
        insta::assert_snapshot!("path_status_de", path_status(Lang::De, &entries));
        insta::assert_snapshot!("path_status_en", path_status(Lang::En, &entries));
    }
}
