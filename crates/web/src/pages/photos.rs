//! Fotos: Zustand, Abgleich und Duplikate (ADR 0012). Duplikate werden erst
//! geprüft und nur nach einer zweiten, ausdrücklichen Bestätigung in den
//! Papierkorb gelegt.

use std::fmt::Write;

use axum::extract::{Form, State};
use axum::response::Response;
use protonsync_api::{
    DuplicateGroupInfo, DuplicateReasonInfo, ErrorCode, PhotoCounts, PhotoInfo, PhotoOutcomeInfo,
    PhotoRunInfo, PhotoStatus, Request, Response as ApiResponse, TrashedPhotoInfo,
};

use super::{
    ApplyForm, Shared, ask, direction, error_page, logged_in_page, require_form_session,
    require_session, status_class, unexpected,
};
use crate::html::{Lang, Nav, button_form, csrf_field, escape, format_time};
use crate::visitor::{Session, Visitor};
use crate::{AppState, Backend};

/// Formularfeld für die Kennung eines ausgewählten Fotos (mehrfach).
const ID_FIELD: &str = "id";

fn title(lang: Lang) -> &'static str {
    lang.pick("Fotos", "Photos")
}

fn not_configured(lang: Lang) -> String {
    format!(
        "<p>{}</p>",
        lang.pick(
            "Der Foto-Abgleich ist nicht eingerichtet (Abschnitt <code>[photos]</code> in der Konfiguration).",
            "Photo sync is not set up (section <code>[photos]</code> in the configuration).",
        )
    )
}

/// Wie `ask`, aber „nicht eingerichtet“ wird zu einem Hinweis statt einer Fehlerseite.
async fn ask_photos<B: Backend>(
    state: &AppState<B>,
    visitor: &Visitor,
    session: &Session,
    request: Request,
) -> Result<Option<ApiResponse>, Response> {
    match state.backend.handle(request).await {
        ApiResponse::Error(error) if error.code == ErrorCode::NotFound => Ok(None),
        ApiResponse::Error(error) => Err(error_page(visitor, session, &error)),
        response => Ok(Some(response)),
    }
}

fn planned_counts(lang: Lang, counts: &PhotoCounts) -> String {
    format!(
        "{} {}, {} {}, {} {}, {} {}",
        counts.downloaded,
        lang.pick("herunterzuladen", "to download"),
        counts.uploaded,
        lang.pick("hochzuladen", "to upload"),
        counts.linked,
        lang.pick("zu verknüpfen", "to link"),
        counts.trashed_local + counts.trashed_remote,
        lang.pick("in Papierkörbe", "to trash"),
    )
}

fn applied_counts(lang: Lang, counts: &PhotoCounts) -> String {
    format!(
        "{} {}, {} {}, {} {}, {} {}, {} {}",
        counts.downloaded,
        lang.pick("heruntergeladen", "downloaded"),
        counts.uploaded,
        lang.pick("hochgeladen", "uploaded"),
        counts.linked,
        lang.pick("verknüpft", "linked"),
        counts.trashed_local + counts.trashed_remote,
        lang.pick("in Papierkörbe", "trashed"),
        counts.failures.len(),
        lang.pick("Fehler", "errors"),
    )
}

fn outcome_text(lang: Lang, outcome: &PhotoOutcomeInfo) -> String {
    match outcome {
        PhotoOutcomeInfo::Planned(counts) => format!(
            "{}: {}",
            lang.pick("Probelauf", "Dry run"),
            planned_counts(lang, counts)
        ),
        PhotoOutcomeInfo::Applied(counts) => applied_counts(lang, counts),
        PhotoOutcomeInfo::Failed { message } => {
            format!("{}: {message}", lang.pick("Fehlgeschlagen", "Failed"))
        }
    }
}

fn is_failed(run: &PhotoRunInfo) -> bool {
    match &run.outcome {
        PhotoOutcomeInfo::Failed { .. } => true,
        PhotoOutcomeInfo::Applied(counts) => !counts.failures.is_empty(),
        PhotoOutcomeInfo::Planned(_) => false,
    }
}

fn status_table<B>(state: &AppState<B>, lang: Lang, status: &PhotoStatus) -> String {
    let count = status.photos.map_or_else(
        || lang.pick("noch nicht gelesen", "not read yet").to_owned(),
        |count| count.to_string(),
    );
    let last = status.last_run.as_ref().map_or_else(
        || format!("<td>{}</td>", lang.pick("noch keiner", "none yet")),
        |run| {
            format!(
                "<td{}>{}: {}</td>",
                status_class(is_failed(run)),
                format_time(run.finished_at, &state.zone),
                escape(&outcome_text(lang, &run.outcome))
            )
        },
    );
    let running = if status.syncing {
        lang.pick(" (läuft)", " (running)")
    } else {
        ""
    };
    format!(
        "<table><tr><th>{}</th><td>{}{running}</td></tr><tr><th>{}</th><td>{}</td></tr>\
         <tr><th>{}</th><td>{count}</td></tr><tr><th>{}</th>{last}</tr>\
         <tr><th>{}</th><td><a href=\"/trash\">{}</a></td></tr></table>",
        lang.pick("Fotoordner", "Photo folder"),
        escape(&status.folder),
        lang.pick("Richtung", "Direction"),
        direction(lang, status.direction),
        lang.pick("Fotos in Proton", "Photos in Proton"),
        lang.pick("Letzter Abgleich", "Last sync"),
        lang.pick("Papierkorb", "Trash"),
        status.trash_entries,
    )
}

fn sync_buttons(lang: Lang, csrf: &str) -> String {
    format!(
        "<p>{}{}</p>",
        button_form("/photos/sync", csrf, &[], lang.pick("Planen", "Plan")),
        button_form(
            "/photos/sync",
            csrf,
            &[("apply", "1")],
            lang.pick("Jetzt abgleichen", "Sync now")
        ),
    )
}

pub(crate) async fn photos<B: Backend>(State(state): Shared<B>, visitor: Visitor) -> Response {
    let session = match require_session(&visitor) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let lang = visitor.lang;
    let body = match ask_photos(&state, &visitor, session, Request::Photos).await {
        Ok(Some(ApiResponse::Photos(status))) => format!(
            "{}{}<p><a href=\"/photos/duplicates\">{}</a></p>",
            status_table(&state, lang, &status),
            sync_buttons(lang, &session.csrf),
            lang.pick("Duplikate suchen", "Find duplicates"),
        ),
        Ok(None) => not_configured(lang),
        Ok(Some(other)) => return unexpected(&visitor, session, &other),
        Err(response) => return response,
    };
    logged_in_page(&visitor, session, Nav::Photos, title(lang), &body)
}

pub(crate) async fn sync<B: Backend>(
    State(state): Shared<B>,
    visitor: Visitor,
    Form(form): Form<ApplyForm>,
) -> Response {
    let session = match require_form_session(&visitor, &form.csrf) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let lang = visitor.lang;
    let request = Request::SyncPhotos {
        apply: form.apply.is_some(),
    };
    let run = match ask(&state, &visitor, session, request).await {
        Ok(ApiResponse::PhotoRun(run)) => run,
        Ok(other) => return unexpected(&visitor, session, &other),
        Err(response) => return response,
    };
    let mut body = format!(
        "<p{}>{}</p>",
        status_class(is_failed(&run)),
        escape(&outcome_text(lang, &run.outcome))
    );
    if let PhotoOutcomeInfo::Planned(_) = run.outcome {
        let _ = write!(
            body,
            "<p class=\"note\">{}</p><p>{}</p>",
            lang.pick(
                "Probelauf, nichts wurde geändert.",
                "Dry run, nothing was changed."
            ),
            button_form(
                "/photos/sync",
                &session.csrf,
                &[("apply", "1")],
                lang.pick("Jetzt abgleichen", "Sync now")
            )
        );
    }
    let _ = write!(
        body,
        "<p><a href=\"/photos\">{}</a></p>",
        lang.pick("Zu den Fotos", "Back to photos")
    );
    logged_in_page(&visitor, session, Nav::Photos, title(lang), &body)
}

fn reason(lang: Lang, reason: DuplicateReasonInfo) -> &'static str {
    match reason {
        DuplicateReasonInfo::SameContent => lang.pick("gleicher Inhalt", "same content"),
        DuplicateReasonInfo::SameNameAndCaptureTime => lang.pick(
            "gleicher Name und gleiche Aufnahmezeit",
            "same name and capture time",
        ),
    }
}

fn photo_row<B>(state: &AppState<B>, lang: Lang, photo: &PhotoInfo, keep: bool) -> String {
    let choice = if keep {
        format!("<strong>{}</strong>", lang.pick("bleibt", "stays"))
    } else {
        // Vorgeschlagen, aber erst die zweite Bestätigung legt es in den Papierkorb.
        format!(
            "<label><input type=\"checkbox\" name=\"{ID_FIELD}\" value=\"{}\" checked> {}</label>",
            escape(&photo.id),
            lang.pick("in den Papierkorb", "to the trash")
        )
    };
    let mut marks = Vec::new();
    if photo.favorite {
        marks.push(lang.pick("Favorit", "favorite").to_owned());
    }
    if photo.albums > 0 {
        marks.push(format!("{} {}", photo.albums, lang.pick("Alben", "albums")));
    }
    format!(
        "<tr><td>{choice}</td><td>{}</td><td>{} KB</td><td>{}</td><td>{}</td></tr>",
        escape(&photo.name),
        photo.size.div_ceil(1024),
        format_time(photo.uploaded, &state.zone),
        escape(&marks.join(", ")),
    )
}

fn duplicate_groups<B>(
    state: &AppState<B>,
    lang: Lang,
    csrf: &str,
    groups: &[DuplicateGroupInfo],
) -> String {
    if groups.is_empty() {
        return format!(
            "<p>{}</p>",
            lang.pick("Keine Duplikate gefunden.", "No duplicates found.")
        );
    }
    let mut body = format!(
        "<p class=\"note\">{}</p><form method=\"post\" action=\"/photos/duplicates/trash\">{}",
        lang.pick(
            "Nur Vorschläge: Nichts wird gelöscht, bevor du die Auswahl prüfst und bestätigst.",
            "Suggestions only: nothing is deleted before you check and confirm the selection.",
        ),
        csrf_field(csrf),
    );
    for group in groups {
        let _ = write!(
            body,
            "<h2>{} ({}, {})</h2><table><tr><th></th><th>{}</th><th>{}</th><th>{}</th><th></th></tr>{}",
            escape(&group.keep.name),
            reason(lang, group.reason),
            format_time(group.captured, &state.zone),
            lang.pick("Name", "Name"),
            lang.pick("Größe", "Size"),
            lang.pick("Hochgeladen", "Uploaded"),
            photo_row(state, lang, &group.keep, true),
        );
        for photo in &group.others {
            body.push_str(&photo_row(state, lang, photo, false));
        }
        body.push_str("</table>");
    }
    let _ = write!(
        body,
        "<p><button>{}</button></p></form>",
        lang.pick("Auswahl prüfen", "Check selection")
    );
    body
}

pub(crate) async fn duplicates<B: Backend>(State(state): Shared<B>, visitor: Visitor) -> Response {
    let session = match require_session(&visitor) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let lang = visitor.lang;
    let body = match ask_photos(&state, &visitor, session, Request::PhotoDuplicates).await {
        Ok(Some(ApiResponse::PhotoDuplicates { groups })) => {
            duplicate_groups(&state, lang, &session.csrf, &groups)
        }
        Ok(None) => not_configured(lang),
        Ok(Some(other)) => return unexpected(&visitor, session, &other),
        Err(response) => return response,
    };
    let title = lang.pick("Foto-Duplikate", "Photo duplicates");
    logged_in_page(&visitor, session, Nav::Photos, title, &body)
}

/// Felder des Formulars: `csrf`, `apply` (optional) und `id` beliebig oft.
/// Ein einfaches `Form<Struct>` kann wiederholte Schlüssel nicht lesen.
struct TrashForm {
    csrf: String,
    apply: bool,
    ids: Vec<String>,
}

impl TrashForm {
    fn parse(fields: Vec<(String, String)>) -> Self {
        let mut form = Self {
            csrf: String::new(),
            apply: false,
            ids: Vec::new(),
        };
        for (name, value) in fields {
            match name.as_str() {
                "csrf" => form.csrf = value,
                "apply" => form.apply = true,
                ID_FIELD => form.ids.push(value),
                _ => {}
            }
        }
        form
    }
}

fn trashed_list(photos: &[TrashedPhotoInfo]) -> String {
    let mut list = String::from("<ul>");
    for photo in photos {
        let _ = write!(list, "<li>{}</li>", escape(&photo.path));
    }
    list.push_str("</ul>");
    list
}

fn confirm_form(lang: Lang, csrf: &str, ids: &[String]) -> String {
    let mut form = format!(
        "<form method=\"post\" action=\"/photos/duplicates/trash\">{}<input type=\"hidden\" name=\"apply\" value=\"1\">",
        csrf_field(csrf)
    );
    for id in ids {
        let _ = write!(
            form,
            "<input type=\"hidden\" name=\"{ID_FIELD}\" value=\"{}\">",
            escape(id)
        );
    }
    let _ = write!(
        form,
        "<button>{}</button></form>",
        lang.pick("In den Papierkorb legen", "Move to the trash")
    );
    form
}

/// Ohne `apply` nur prüfen und zur Bestätigung anzeigen; mit `apply` in den Papierkorb.
pub(crate) async fn trash<B: Backend>(
    State(state): Shared<B>,
    visitor: Visitor,
    Form(fields): Form<Vec<(String, String)>>,
) -> Response {
    let form = TrashForm::parse(fields);
    let session = match require_form_session(&visitor, &form.csrf) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let lang = visitor.lang;
    let back = format!(
        "<p><a href=\"/photos/duplicates\">{}</a></p>",
        lang.pick("Zu den Duplikaten", "Back to duplicates")
    );
    if form.ids.is_empty() {
        let body = format!(
            "<p>{}</p>{back}",
            lang.pick("Keine Fotos ausgewählt.", "No photos selected.")
        );
        return logged_in_page(&visitor, session, Nav::Photos, title(lang), &body);
    }
    let request = Request::TrashPhotoDuplicates {
        ids: form.ids.clone(),
        apply: form.apply,
    };
    let (dry_run, photos) = match ask(&state, &visitor, session, request).await {
        Ok(ApiResponse::PhotosTrashed { dry_run, photos }) => (dry_run, photos),
        Ok(other) => return unexpected(&visitor, session, &other),
        Err(response) => return response,
    };
    let body = if dry_run {
        format!(
            "<p class=\"note\">{}</p>{}{}{back}",
            lang.pick(
                "Diese Fotos würden in den Papierkorb gelegt (dort bleiben sie bis zum Ablauf der Frist wiederherstellbar):",
                "These photos would be moved to the trash (they stay restorable there until the retention period ends):",
            ),
            trashed_list(&photos),
            confirm_form(lang, &session.csrf, &form.ids),
        )
    } else {
        format!(
            "<p>{}</p>{}<p><a href=\"/trash\">{}</a></p>",
            lang.pick("In den Papierkorb gelegt:", "Moved to the trash:"),
            trashed_list(&photos),
            lang.pick("Zum Papierkorb", "Open the trash"),
        )
    };
    logged_in_page(&visitor, session, Nav::Photos, title(lang), &body)
}
