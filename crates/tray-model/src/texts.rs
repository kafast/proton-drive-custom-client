//! Fixed UI texts in both languages. QML looks them up via
//! `tray.tr("key")`; a test checks that every key used in the QML files
//! is listed here.

use protonsync_client::Lang;

/// Key, German text, English text.
const TEXTS: &[(&str, &str, &str)] = &[
    ("app.name", "protonsync", "protonsync"),
    ("menu.sync_now", "Jetzt abgleichen", "Sync now"),
    ("menu.plan_now", "Probelauf starten", "Start dry run"),
    ("menu.status", "Status …", "Status …"),
    ("menu.conflicts", "Konflikte lösen …", "Resolve conflicts …"),
    ("menu.login", "Anmelden …", "Log in …"),
    ("menu.logout", "Abmelden", "Log out"),
    ("menu.settings", "Einstellungen …", "Settings …"),
    ("menu.quit", "Tray-App beenden", "Quit tray app"),
    ("tab.overview", "Übersicht", "Overview"),
    ("tab.conflicts", "Konflikte", "Conflicts"),
    ("tab.settings", "Einstellungen", "Settings"),
    ("overview.account", "Konto", "Account"),
    (
        "overview.not_logged_in",
        "Nicht angemeldet",
        "Not logged in",
    ),
    ("overview.dry_run_title", "Probelauf-Modus", "Dry-run mode"),
    (
        "overview.dry_run_hint",
        "protonsync plant nur und ändert nichts, weder lokal noch in Proton. Einmalig abgleichen kannst du im Terminal:",
        "protonsync only plans and changes nothing, neither locally nor in Proton. To sync once, use a terminal:",
    ),
    (
        "overview.cmd_plan",
        "nur planen: zeigt, was passieren würde",
        "plan only: shows what would happen",
    ),
    (
        "overview.cmd_apply",
        "einmal wirklich abgleichen (alle Ordner)",
        "really sync once (all folders)",
    ),
    (
        "overview.cmd_apply_pair",
        "nur diesen einen Sync-Ordner abgleichen",
        "sync only this one folder",
    ),
    (
        "overview.dry_run_off",
        "Dauerhaft abgleichen: unter Einstellungen den Modus auf „Aktiv“ stellen (oder protonsync config set --dry-run false). Das gilt sofort.",
        "Sync permanently: set the mode to “Active” under Settings (or protonsync config set --dry-run false). This applies right away.",
    ),
    ("overview.pairs", "Sync-Ordner", "Sync folders"),
    (
        "overview.no_pairs",
        "Keine Sync-Ordner konfiguriert.",
        "No sync folders configured.",
    ),
    ("overview.mounts", "Laufwerke", "Drives"),
    ("overview.open_folder", "Ordner öffnen", "Open folder"),
    (
        "overview.daemon_offline",
        "Der Daemon läuft nicht. Starten mit: systemctl --user start protonsyncd",
        "The daemon is not running. Start it with: systemctl --user start protonsyncd",
    ),
    ("help.title", "Was kann ich tun?", "What can I do?"),
    (
        "help.actions",
        "<ul>\
<li>Dateien einfach in einen Sync-Ordner legen, ändern oder löschen: protonsync gleicht automatisch mit Proton ab. <b>Ordner öffnen</b> zeigt den Ordner im Dateimanager.</li>\
<li>Reiter <b>Konflikte</b>: entscheiden, welche Version bleibt, wenn eine Datei auf beiden Seiten geändert wurde.</li>\
<li>Reiter <b>Einstellungen</b>: Sprache, Benachrichtigungen, Autostart und die Konfigurationsdatei.</li>\
<li>Rechtsklick auf die Wolke: <b>Jetzt abgleichen</b>, Anmelden/Abmelden, Tray-App beenden.</li>\
<li>In Laufwerken im Dateimanager (Dolphin/Nautilus) per Rechtsklick: <b>Immer lokal halten</b> oder <b>Speicher freigeben</b>.</li>\
<li>Gelöschtes liegt erst im Papierkorb von protonsync und lässt sich wiederherstellen.</li>\
</ul>",
        "<ul>\
<li>Simply put, change or delete files in a sync folder: protonsync syncs them with Proton automatically. <b>Open folder</b> shows the folder in the file manager.</li>\
<li><b>Conflicts</b> tab: decide which version stays when a file was changed on both sides.</li>\
<li><b>Settings</b> tab: language, notifications, autostart and the configuration file.</li>\
<li>Right-click the cloud: <b>Sync now</b>, log in/out, quit the tray app.</li>\
<li>In drives, right-click in the file manager (Dolphin/Nautilus): <b>Always keep local</b> or <b>Free up space</b>.</li>\
<li>Deleted files go to protonsync's trash first and can be restored.</li>\
</ul>",
    ),
    (
        "help.commands",
        "Weitere Befehle im Terminal (Zustand, Konflikte, Papierkorb, Hilfe):",
        "More commands in a terminal (state, conflicts, trash, help):",
    ),
    ("help.open_manual", "Handbuch öffnen", "Open manual"),
    ("help.open_commands", "Alle Befehle", "All commands"),
    (
        "help.manual_url",
        "https://github.com/kafast/proton-drive-custom-client/blob/main/docs/handbuch.md#66-tray-app",
        "https://github.com/kafast/proton-drive-custom-client/blob/main/docs/manual.md#66-tray-app",
    ),
    (
        "help.commands_url",
        "https://github.com/kafast/proton-drive-custom-client/blob/main/docs/handbuch.md#61-befehle",
        "https://github.com/kafast/proton-drive-custom-client/blob/main/docs/manual.md#61-commands",
    ),
    (
        "help.local_manual",
        "Mit dem Paket installiert liegt das Handbuch auch unter /usr/share/doc/protonsync/.",
        "When installed as a package, the manual is also in /usr/share/doc/protonsync/.",
    ),
    (
        "conflicts.none",
        "Keine offenen Konflikte.",
        "No open conflicts.",
    ),
    (
        "conflicts.hint",
        "Beide Versionen sind erhalten. Die verworfene Version landet im Papierkorb.",
        "Both versions are kept. The discarded version goes to the trash.",
    ),
    ("conflicts.keep_local", "Lokale behalten", "Keep local"),
    (
        "conflicts.keep_remote",
        "Proton-Version behalten",
        "Keep Proton version",
    ),
    ("conflicts.keep_both", "Beide behalten", "Keep both"),
    ("settings.tray", "Tray-App", "Tray app"),
    ("settings.language", "Sprache", "Language"),
    ("settings.language_auto", "Wie das System", "Same as system"),
    (
        "settings.notifications",
        "Benachrichtigungen anzeigen",
        "Show notifications",
    ),
    (
        "settings.autostart",
        "Beim Anmelden starten",
        "Start at login",
    ),
    ("settings.daemon", "Daemon", "Daemon"),
    ("settings.daemon_version", "Version", "Version"),
    ("settings.mode", "Modus", "Mode"),
    (
        "settings.mode_dry_run",
        "Probelauf (nur planen)",
        "Dry run (plan only)",
    ),
    ("settings.mode_active", "Aktiv", "Active"),
    (
        "settings.config_file",
        "Konfigurationsdatei",
        "Configuration file",
    ),
    (
        "settings.open_config",
        "Konfiguration öffnen",
        "Open configuration",
    ),
    (
        "settings.restart_hint",
        "Die Einstellungen oben gelten sofort und werden in der Konfiguration gespeichert. Alles andere (z. B. Upload-Jobs, neue Sync-Ordner) dort ändern und den Daemon neu starten: systemctl --user restart protonsyncd",
        "The settings above apply right away and are saved in the configuration. Change everything else (e.g. upload jobs, new sync folders) there and restart the daemon: systemctl --user restart protonsyncd",
    ),
    (
        "settings.trash_days",
        "Papierkorb leeren nach (Tagen)",
        "Empty trash after (days)",
    ),
    (
        "settings.not_editable",
        "Der Daemon kann die Konfigurationsdatei nicht schreiben; Einstellungen nur dort ändern.",
        "The daemon cannot write the configuration file; change settings there.",
    ),
    ("settings.pairs", "Sync-Ordner", "Sync folders"),
    ("settings.direction", "Richtung", "Direction"),
    (
        "settings.protect_remote",
        "Löschschutz: lokal Gelöschtes und Ersetztes nie in Proton löschen (protect_remote)",
        "Protection: never delete or replace anything in Proton (protect_remote)",
    ),
    (
        "settings.archive_versions",
        "Versionsarchiv: Gelöschtes und Überschriebenes beidseitig nach _archiv (archive_versions)",
        "Version archive: move deleted and replaced versions to _archiv on both sides (archive_versions)",
    ),
    ("login.title", "Bei Proton anmelden", "Log in to Proton"),
    (
        "login.username",
        "Benutzername oder E-Mail",
        "Username or email",
    ),
    ("login.password", "Passwort", "Password"),
    (
        "login.code",
        "2FA-Code (Authenticator-App)",
        "2FA code (authenticator app)",
    ),
    (
        "login.data_password",
        "Mailbox-Passwort",
        "Mailbox password",
    ),
    ("login.submit", "Weiter", "Continue"),
    ("login.cancel", "Abbrechen", "Cancel"),
    (
        "login.hint",
        "Gespeichert werden nur Sitzungsschlüssel, nie das Passwort.",
        "Only session keys are stored, never the password.",
    ),
    ("common.close", "Schließen", "Close"),
];

/// Text for `key` in `lang`, or `None` for an unknown key.
#[must_use]
pub fn text(key: &str, lang: Lang) -> Option<&'static str> {
    TEXTS
        .iter()
        .find(|(candidate, _, _)| *candidate == key)
        .map(|(_, de, en)| lang.pick(de, en))
}

/// Text for `key`; unknown keys show up visibly as `[key]`.
#[must_use]
pub fn text_or_key(key: &str, lang: Lang) -> String {
    text(key, lang).map_or_else(|| format!("[{key}]"), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn keys_are_unique_and_texts_filled() {
        let mut keys = BTreeSet::new();
        for (key, de, en) in TEXTS {
            assert!(keys.insert(*key), "duplicate: {key}");
            assert!(!de.is_empty() && !en.is_empty(), "{key}");
        }
    }

    #[test]
    fn looks_up_both_languages() {
        assert_eq!(text("tab.conflicts", Lang::De), Some("Konflikte"));
        assert_eq!(text("tab.conflicts", Lang::En), Some("Conflicts"));
        assert_eq!(text("does.not.exist", Lang::De), None);
        assert_eq!(text_or_key("does.not.exist", Lang::En), "[does.not.exist]");
    }

    /// All keys that the tray app's QML files use via `t("…")`.
    fn keys_used_in_qml() -> BTreeSet<String> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../gui-qt/qml");
        let mut keys = BTreeSet::new();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            // `t("…")` as a separate word, not e.g. `split("…")`.
            for (start, _) in text.match_indices("t(\"") {
                let before = text[..start].chars().next_back();
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let rest = &text[start + 3..];
                if let Some(end) = rest.find('"') {
                    keys.insert(rest[..end].to_owned());
                }
            }
        }
        keys
    }

    #[test]
    fn every_key_used_in_qml_exists() {
        let used = keys_used_in_qml();
        assert!(!used.is_empty(), "no QML files found");
        let missing: Vec<_> = used
            .iter()
            .filter(|key| text(key, Lang::De).is_none())
            .collect();
        assert!(missing.is_empty(), "missing texts: {missing:?}");
    }
}
