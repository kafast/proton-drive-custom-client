//! Settings: the editable ones as a form (ADR 0021), upload jobs read-only.
//!
//! The form sends every field; only the values that differ from the current
//! ones go to the daemon, so the configuration file only changes where the
//! user changed something.

use std::fmt::Write;

use axum::extract::{Form, State};
use axum::response::Response;
use protonsync_api::{
    Direction, JobInfo, JobVerification, PairSettings, PairSettingsChange, Request,
    Response as ApiResponse, SettingsChange, SettingsInfo,
};

use super::{
    Shared, ask, direction, load_jobs, logged_in_page, require_form_session, require_session,
    unexpected,
};
use crate::html::{Lang, Nav, csrf_field, escape};
use crate::visitor::{Session, Visitor};
use crate::{AppState, Backend};

const DRY_RUN: &str = "dry_run";
const RETENTION: &str = "trash_retention_days";
const DIRECTION: &str = "direction";
const PROTECT_REMOTE: &str = "protect_remote";
const ARCHIVE_VERSIONS: &str = "archive_versions";
const TRUE: &str = "true";
const FALSE: &str = "false";

const DIRECTIONS: [Direction; 4] = [
    Direction::TwoWay,
    Direction::DownloadOnly,
    Direction::UploadOnly,
    Direction::BackupOnly,
];

/// The value of a direction in the form (as in the configuration file).
fn direction_key(direction: Direction) -> &'static str {
    match direction {
        Direction::TwoWay => "two_way",
        Direction::DownloadOnly => "download_only",
        Direction::UploadOnly => "upload_only",
        Direction::BackupOnly => "backup_only",
    }
}

fn parse_direction(key: &str) -> Option<Direction> {
    DIRECTIONS
        .into_iter()
        .find(|direction| direction_key(*direction) == key)
}

fn parse_bool(text: &str) -> Option<bool> {
    match text {
        TRUE => Some(true),
        FALSE => Some(false),
        _ => None,
    }
}

/// Form field of a pair option, e.g. `direction.main`.
fn pair_field(name: &str, pair: &str) -> String {
    format!("{name}.{pair}")
}

fn title(lang: Lang) -> &'static str {
    lang.pick("Einstellungen", "Settings")
}

/// A drop-down list; `options` are (value, label).
fn select(name: &str, options: &[(&str, &str)], current: &str, disabled: bool) -> String {
    let mut out = format!(
        "<select name=\"{}\"{}>",
        escape(name),
        if disabled { " disabled" } else { "" }
    );
    for (value, label) in options {
        let _ = write!(
            out,
            "<option value=\"{}\"{}>{}</option>",
            escape(value),
            if *value == current { " selected" } else { "" },
            escape(label)
        );
    }
    out.push_str("</select>");
    out
}

fn yes_no_select(lang: Lang, name: &str, current: bool, disabled: bool) -> String {
    select(
        name,
        &[
            (FALSE, lang.pick("nein", "no")),
            (TRUE, lang.pick("ja", "yes")),
        ],
        if current { TRUE } else { FALSE },
        disabled,
    )
}

fn bool_key(value: bool) -> &'static str {
    if value { TRUE } else { FALSE }
}

pub(crate) async fn settings<B: Backend>(State(state): Shared<B>, visitor: Visitor) -> Response {
    let session = match require_session(&visitor) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let current = match load_settings(&state, &visitor, session).await {
        Ok(current) => current,
        Err(response) => return response,
    };
    settings_page(&state, &visitor, session, &current, None).await
}

/// Saves the form; shows the page again with a note.
pub(crate) async fn save<B: Backend>(
    State(state): Shared<B>,
    visitor: Visitor,
    Form(fields): Form<Vec<(String, String)>>,
) -> Response {
    let csrf = field(&fields, "csrf").unwrap_or_default();
    let session = match require_form_session(&visitor, csrf) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let lang = visitor.lang;
    let current = match load_settings(&state, &visitor, session).await {
        Ok(current) => current,
        Err(response) => return response,
    };
    let change = match changes(&current, &fields) {
        Ok(change) => change,
        Err(message) => {
            return settings_page(&state, &visitor, session, &current, Some(Err(message))).await;
        }
    };
    if change.is_empty() {
        let note = lang.pick("Nichts geändert.", "Nothing changed.").to_owned();
        return settings_page(&state, &visitor, session, &current, Some(Ok(note))).await;
    }
    let saved = match ask(
        &state,
        &visitor,
        session,
        Request::ChangeSettings { change },
    )
    .await
    {
        Ok(ApiResponse::Settings(saved)) => saved,
        Ok(other) => return unexpected(&visitor, session, &other),
        Err(response) => return response,
    };
    let note = lang
        .pick(
            "Gespeichert und sofort wirksam.",
            "Saved; applies right away.",
        )
        .to_owned();
    settings_page(&state, &visitor, session, &saved, Some(Ok(note))).await
}

async fn load_settings<B: Backend>(
    state: &AppState<B>,
    visitor: &Visitor,
    session: &Session,
) -> Result<SettingsInfo, Response> {
    match ask(state, visitor, session, Request::Settings).await? {
        ApiResponse::Settings(settings) => Ok(settings),
        other => Err(unexpected(visitor, session, &other)),
    }
}

fn field<'a>(fields: &'a [(String, String)], name: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// The fields that differ from `current`. Missing fields stay as they are.
fn changes(current: &SettingsInfo, fields: &[(String, String)]) -> Result<SettingsChange, String> {
    let invalid = |name: &str| format!("invalid value for {name}");
    let mut change = SettingsChange::default();
    if let Some(text) = field(fields, DRY_RUN) {
        let dry_run = parse_bool(text).ok_or_else(|| invalid(DRY_RUN))?;
        change.dry_run = (dry_run != current.dry_run).then_some(dry_run);
    }
    if let Some(text) = field(fields, RETENTION) {
        let days: u32 = text.trim().parse().map_err(|_| invalid(RETENTION))?;
        change.trash_retention_days = (days != current.trash_retention_days).then_some(days);
    }
    for pair in &current.pairs {
        let pair_change = pair_changes(pair, fields)?;
        if pair_change.direction.is_some()
            || pair_change.protect_remote.is_some()
            || pair_change.archive_versions.is_some()
        {
            change.pairs.push(pair_change);
        }
    }
    Ok(change)
}

fn pair_changes(
    pair: &PairSettings,
    fields: &[(String, String)],
) -> Result<PairSettingsChange, String> {
    let read_bool = |name: &str, current: bool| -> Result<Option<bool>, String> {
        let key = pair_field(name, &pair.id);
        match field(fields, &key) {
            None => Ok(None),
            Some(text) => {
                let value = parse_bool(text).ok_or_else(|| format!("invalid value for {key}"))?;
                Ok((value != current).then_some(value))
            }
        }
    };
    let key = pair_field(DIRECTION, &pair.id);
    let direction = match field(fields, &key) {
        None => None,
        Some(text) => {
            let direction =
                parse_direction(text).ok_or_else(|| format!("invalid value for {key}"))?;
            (direction != pair.direction).then_some(direction)
        }
    };
    Ok(PairSettingsChange {
        direction,
        protect_remote: read_bool(PROTECT_REMOTE, pair.protect_remote)?,
        archive_versions: read_bool(ARCHIVE_VERSIONS, pair.archive_versions)?,
        ..PairSettingsChange::new(pair.id.clone())
    })
}

/// The page; `note` is the result of a save (`Err`: a message about the form).
async fn settings_page<B: Backend>(
    state: &AppState<B>,
    visitor: &Visitor,
    session: &Session,
    settings: &SettingsInfo,
    note: Option<Result<String, String>>,
) -> Response {
    let lang = visitor.lang;
    let jobs = match load_jobs(state, visitor, session).await {
        Ok(jobs) => jobs,
        Err(response) => return response,
    };
    let mut body = String::new();
    match note {
        Some(Ok(text)) => {
            let _ = write!(body, "<p class=\"note\">{}</p>", escape(&text));
        }
        Some(Err(text)) => {
            let _ = write!(body, "<p class=\"error\">{}</p>", escape(&text));
        }
        None => {}
    }
    body.push_str(&settings_form(lang, &session.csrf, settings));
    body.push_str(&jobs_table(lang, &jobs));
    logged_in_page(visitor, session, Nav::Settings, title(lang), &body)
}

fn settings_form(lang: Lang, csrf: &str, settings: &SettingsInfo) -> String {
    let disabled = !settings.editable;
    let mut out = format!(
        "<p class=\"muted\">{} <code>{}</code></p>",
        lang.pick("Konfigurationsdatei:", "Configuration file:"),
        escape(&settings.config_path)
    );
    if disabled {
        let _ = write!(
            out,
            "<p class=\"note\">{}</p>",
            lang.pick(
                "Der Daemon kann die Konfigurationsdatei nicht schreiben; Einstellungen nur dort ändern und den Daemon neu starten.",
                "The daemon cannot write the configuration file; change settings there and restart the daemon.",
            )
        );
    }
    let _ = write!(
        out,
        "<form method=\"post\" action=\"/settings\" class=\"settings\">{}<h2>{}</h2><table>",
        csrf_field(csrf),
        lang.pick("Allgemein", "General")
    );
    let _ = write!(
        out,
        "<tr><th>{}</th><td>{}</td><td class=\"muted\">{}</td></tr>",
        lang.pick("Modus", "Mode"),
        select(
            DRY_RUN,
            &[
                (
                    TRUE,
                    lang.pick("Probelauf (nur planen)", "Dry run (plan only)")
                ),
                (
                    FALSE,
                    lang.pick("Aktiv (Änderungen ausführen)", "Active (apply changes)")
                ),
            ],
            bool_key(settings.dry_run),
            disabled,
        ),
        lang.pick(
            "Gilt für automatische Abgleiche, Upload-Jobs und das Leeren des Papierkorbs.",
            "Applies to automatic syncs, upload jobs and emptying the trash.",
        )
    );
    let _ = write!(
        out,
        "<tr><th>{}</th><td><input type=\"number\" name=\"{RETENTION}\" min=\"1\" value=\"{}\"{}></td><td class=\"muted\">{}</td></tr></table>",
        lang.pick("Papierkorb leeren nach (Tagen)", "Empty trash after (days)"),
        settings.trash_retention_days,
        if disabled { " disabled" } else { "" },
        lang.pick("Mindestens 1 Tag.", "At least 1 day."),
    );
    let _ = write!(out, "<h2>{}</h2>", lang.pick("Sync-Paare", "Sync pairs"));
    if settings.pairs.is_empty() {
        let _ = write!(
            out,
            "<p>{}</p>",
            lang.pick(
                "Keine Sync-Paare konfiguriert (Abschnitt <code>[[pair]]</code>).",
                "No sync pairs configured (section <code>[[pair]]</code>).",
            )
        );
    } else {
        let _ = write!(
            out,
            "<table><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr>",
            lang.pick("Paar", "Pair"),
            lang.pick("Richtung", "Direction"),
            lang.pick(
                "Löschschutz für Proton (protect_remote)",
                "Protect Proton (protect_remote)"
            ),
            lang.pick(
                "Versionsarchiv (archive_versions)",
                "Version archive (archive_versions)"
            ),
        );
        for pair in &settings.pairs {
            out.push_str(&pair_row(lang, pair, disabled));
        }
        out.push_str("</table>");
    }
    if !disabled {
        let _ = write!(
            out,
            "<p><button>{}</button></p>",
            lang.pick("Speichern", "Save")
        );
    }
    out.push_str("</form>");
    out
}

fn pair_row(lang: Lang, pair: &PairSettings, disabled: bool) -> String {
    let directions: Vec<(&str, &str)> = DIRECTIONS
        .into_iter()
        .map(|d| (direction_key(d), direction(lang, d)))
        .collect();
    format!(
        "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
        escape(&pair.id),
        select(
            &pair_field(DIRECTION, &pair.id),
            &directions,
            direction_key(pair.direction),
            disabled
        ),
        yes_no_select(
            lang,
            &pair_field(PROTECT_REMOTE, &pair.id),
            pair.protect_remote,
            disabled
        ),
        yes_no_select(
            lang,
            &pair_field(ARCHIVE_VERSIONS, &pair.id),
            pair.archive_versions,
            disabled
        ),
    )
}

fn jobs_table(lang: Lang, jobs: &[JobInfo]) -> String {
    let mut out = format!(
        "<h2>{}</h2><p class=\"muted\">{}</p><table><tr><th>id</th><th>source</th><th>pattern</th><th>target</th><th>schedule</th><th>keep_last</th><th>max_age_days</th><th>verify</th></tr>",
        lang.pick("Upload-Jobs", "Upload jobs"),
        lang.pick(
            "Upload-Jobs werden in der Konfigurationsdatei (<code>config.toml</code>) geändert; danach den Daemon neu starten.",
            "Upload jobs are changed in the configuration file (<code>config.toml</code>); restart the daemon afterwards.",
        ),
    );
    for job in jobs {
        let optional = |value: Option<u32>| value.map_or_else(|| "–".to_owned(), |v| v.to_string());
        let _ = write!(
            out,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape(&job.id),
            escape(&job.source),
            escape(&job.pattern),
            escape(&job.target),
            escape(&job.schedule),
            optional(job.keep_last),
            optional(job.max_age_days),
            match job.verify {
                JobVerification::Size => "size",
                JobVerification::Hash => "hash",
            }
        );
    }
    out.push_str("</table>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn current() -> SettingsInfo {
        SettingsInfo {
            config_path: "/c.toml".into(),
            editable: true,
            dry_run: true,
            trash_retention_days: 30,
            pairs: vec![PairSettings {
                id: "main".into(),
                direction: Direction::TwoWay,
                protect_remote: false,
                archive_versions: false,
            }],
        }
    }

    fn fields(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn only_differences_are_sent() {
        let unchanged = fields(&[
            ("dry_run", "true"),
            ("trash_retention_days", "30"),
            ("direction.main", "two_way"),
            ("protect_remote.main", "false"),
            ("archive_versions.main", "false"),
        ]);
        assert!(changes(&current(), &unchanged).unwrap().is_empty());

        let changed = fields(&[
            ("dry_run", "false"),
            ("trash_retention_days", "30"),
            ("direction.main", "backup_only"),
            ("protect_remote.main", "false"),
            ("archive_versions.main", "true"),
        ]);
        assert_eq!(
            changes(&current(), &changed).unwrap(),
            SettingsChange {
                dry_run: Some(false),
                trash_retention_days: None,
                pairs: vec![PairSettingsChange {
                    direction: Some(Direction::BackupOnly),
                    archive_versions: Some(true),
                    ..PairSettingsChange::new("main")
                }],
            }
        );
    }

    #[test]
    fn invalid_values_are_reported() {
        for (name, value) in [
            ("dry_run", "maybe"),
            ("trash_retention_days", "many"),
            ("direction.main", "sideways"),
            ("protect_remote.main", "1"),
        ] {
            let error = changes(&current(), &fields(&[(name, value)])).unwrap_err();
            assert!(error.contains(name), "{error}");
        }
    }

    #[test]
    fn a_read_only_file_disables_the_form() {
        let settings = SettingsInfo {
            editable: false,
            ..current()
        };
        let form = settings_form(Lang::En, "token", &settings);
        assert!(
            form.contains("cannot write the configuration file"),
            "{form}"
        );
        assert!(
            form.contains("<select name=\"dry_run\" disabled>"),
            "{form}"
        );
        assert!(!form.contains("<button>"), "{form}");
    }
}
