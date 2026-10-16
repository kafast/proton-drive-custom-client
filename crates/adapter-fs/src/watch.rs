//! Lokale Änderungserkennung über inotify (Crate `notify`).
//!
//! Der Watcher meldet nur, *dass* sich etwas geändert hat; was genau, ermittelt
//! der nächste Abgleich selbst. Eigene Hilfsdateien (Präfix
//! [`INTERNAL_PREFIX`](crate::INTERNAL_PREFIX)) und reine Lesezugriffe lösen
//! nichts aus, sonst würde jeder Abgleich gleich den nächsten anstoßen.

use std::path::Path;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use protonsync_application::ports::LocalError;

use crate::INTERNAL_PREFIX;

/// Beobachtet einen Ordner rekursiv, solange der Wert lebt.
pub struct LocalWatcher {
    // Wird nur gehalten: Beim Verwerfen endet die Beobachtung (wie `close()` in Java).
    _watcher: RecommendedWatcher,
}

impl LocalWatcher {
    /// Startet die Beobachtung von `root`. `on_change` läuft auf einem eigenen
    /// Thread und sollte nur kurz ein Signal setzen.
    ///
    /// # Errors
    /// [`LocalError::Io`], wenn inotify nicht verfügbar ist (z. B. Limit erreicht).
    pub fn start(root: &Path, on_change: impl Fn() + Send + 'static) -> Result<Self, LocalError> {
        let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
            // Fehler des Watchers (z. B. Überlauf) als Änderung behandeln: Ein
            // zusätzlicher Abgleich ist harmlos, ein verpasster nicht.
            if result.as_ref().map_or(true, is_relevant) {
                on_change();
            }
        })
        .map_err(|e| LocalError::Io(e.to_string()))?;
        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(|e| LocalError::Io(e.to_string()))?;
        Ok(Self { _watcher: watcher })
    }
}

fn is_relevant(event: &Event) -> bool {
    !matches!(event.kind, EventKind::Access(_)) && event.paths.iter().any(|path| !is_internal(path))
}

fn is_internal(path: &Path) -> bool {
    path.components().any(|part| {
        part.as_os_str()
            .to_string_lossy()
            .starts_with(INTERNAL_PREFIX)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn internal_paths_and_reads_are_ignored() {
        assert!(is_internal(Path::new("/x/.protonsync-trash/a")));
        assert!(!is_internal(Path::new("/x/a/b")));
        let read =
            Event::new(EventKind::Access(notify::event::AccessKind::Any)).add_path("/x/a".into());
        assert!(!is_relevant(&read));
        let write = Event::new(EventKind::Any).add_path("/x/a".into());
        assert!(is_relevant(&write));
        let internal = Event::new(EventKind::Any).add_path("/x/.protonsync-tmp-1".into());
        assert!(!is_relevant(&internal));
    }

    #[test]
    fn reports_new_files() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let _watcher = LocalWatcher::start(dir.path(), move || {
            let _ = tx.send(());
        })
        .unwrap();
        std::fs::write(dir.path().join("neu.txt"), b"x").unwrap();
        assert!(rx.recv_timeout(WAIT).is_ok());
    }
}
