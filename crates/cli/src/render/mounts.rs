//! Textausgabe für FUSE-Laufwerke (Platzhalter-Modus).

use std::fmt::Write;

use protonsync_api::{FreeResult, MountInfo, PinChoice};

use crate::i18n::{Lang, format_bytes as human_bytes};

fn pin_label(lang: Lang, state: PinChoice) -> &'static str {
    match state {
        PinChoice::Pinned => lang.pick("immer lokal", "always local"),
        PinChoice::Unpinned => lang.pick("nur bei Bedarf", "on demand"),
    }
}

/// Übersicht aller Laufwerke.
#[must_use]
pub fn mounts(lang: Lang, mounts: &[MountInfo]) -> String {
    if mounts.is_empty() {
        return format!(
            "{}\n",
            lang.pick(
                "Keine Laufwerke konfiguriert ([[mount]] in der Konfiguration).",
                "No drives configured ([[mount]] in the configuration)."
            )
        );
    }
    let mut out = String::new();
    for mount in mounts {
        let remote = if mount.remote_path.is_empty() {
            lang.pick("Meine Dateien", "My files").to_owned()
        } else {
            mount.remote_path.clone()
        };
        let state = if mount.mounted {
            lang.pick("eingehängt", "mounted")
        } else if mount.cache.is_some() {
            lang.pick("verbunden, nicht eingehängt", "connected, not mounted")
        } else {
            lang.pick(
                "nicht verbunden, erst anmelden",
                "not connected, log in first",
            )
        };
        let _ = writeln!(
            out,
            "[{}] {} <- {remote} ({state})",
            mount.id, mount.mountpoint
        );
        let default = if mount.keep_local {
            PinChoice::Pinned
        } else {
            PinChoice::Unpinned
        };
        let _ = writeln!(
            out,
            "  {}: {}",
            lang.pick("Voreinstellung", "Default"),
            pin_label(lang, default)
        );
        if let Some(cache) = mount.cache {
            let _ = writeln!(
                out,
                "  {}: {} {}, {}",
                lang.pick("Zwischenspeicher", "Cache"),
                cache.files,
                lang.pick("Dateien", "files"),
                human_bytes(cache.bytes)
            );
            if cache.pending_uploads > 0 {
                let _ = writeln!(
                    out,
                    "  {}: {}",
                    lang.pick("Noch hochzuladen", "Pending uploads"),
                    cache.pending_uploads
                );
            }
        }
        for pin in &mount.pins {
            let path = if pin.path.is_empty() { "/" } else { &pin.path };
            let _ = writeln!(out, "  {path}: {}", pin_label(lang, pin.state));
        }
    }
    out
}

/// Ergebnis von `free`.
#[must_use]
pub fn freed(lang: Lang, results: &[FreeResult]) -> String {
    if results.is_empty() {
        return format!(
            "{}\n",
            lang.pick(
                "Kein Laufwerk verbunden; nichts freigegeben.",
                "No drive connected; nothing freed."
            )
        );
    }
    let mut out = String::new();
    for result in results {
        let verb = if result.dry_run {
            lang.pick("würde freigeben", "would free")
        } else {
            lang.pick("freigegeben", "freed")
        };
        let _ = writeln!(
            out,
            "[{}] {verb}: {} {}, {}",
            result.mount,
            result.files.len(),
            lang.pick("Dateien", "files"),
            human_bytes(result.bytes)
        );
        for file in &result.files {
            let _ = writeln!(out, "  {file}");
        }
        if result.skipped > 0 {
            let _ = writeln!(
                out,
                "  {}: {}",
                lang.pick(
                    "Behalten (angeheftet, geändert oder geöffnet)",
                    "Kept (pinned, changed or open)"
                ),
                result.skipped
            );
        }
    }
    if results.iter().any(|result| result.dry_run) {
        let _ = writeln!(
            out,
            "{}",
            lang.pick(
                "Probelauf: Mit --apply wirklich freigeben.",
                "Dry run: use --apply to actually free space."
            )
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use protonsync_api::{CacheInfo, PinInfo};

    fn samples() -> Vec<MountInfo> {
        vec![
            MountInfo {
                id: "home".into(),
                mountpoint: "/home/k/ProtonDrive".into(),
                remote_path: String::new(),
                mounted: true,
                keep_local: false,
                cache: Some(CacheInfo {
                    files: 12,
                    bytes: 1_500_000_000,
                    pending_uploads: 1,
                }),
                pins: vec![PinInfo {
                    path: "Fotos".into(),
                    state: PinChoice::Pinned,
                }],
            },
            MountInfo {
                id: "archiv".into(),
                mountpoint: "/home/k/Archiv".into(),
                remote_path: "Archiv".into(),
                mounted: false,
                keep_local: true,
                cache: None,
                pins: vec![],
            },
        ]
    }

    fn free_sample(dry_run: bool) -> Vec<FreeResult> {
        vec![FreeResult {
            mount: "home".into(),
            dry_run,
            files: vec!["Videos/a.mkv".into()],
            bytes: 300_000_000,
            skipped: 2,
        }]
    }

    #[test]
    fn mount_snapshots() {
        insta::assert_snapshot!("mounts_de", mounts(Lang::De, &samples()));
        insta::assert_snapshot!("mounts_en", mounts(Lang::En, &samples()));
        insta::assert_snapshot!("mounts_none_de", mounts(Lang::De, &[]));
    }

    #[test]
    fn free_snapshots() {
        insta::assert_snapshot!("free_dry_run_de", freed(Lang::De, &free_sample(true)));
        insta::assert_snapshot!("free_applied_en", freed(Lang::En, &free_sample(false)));
    }
}
