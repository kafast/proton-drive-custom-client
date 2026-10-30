//! Change log: what the syncs changed, locally and in Proton (ADR 0024).

use std::fmt::Write;

use axum::extract::{Query, State};
use axum::response::Response;
use protonsync_api::{ChangeInfo, ChangedSideInfo, Request, Response as ApiResponse};
use protonsync_client::changes::describe;
use serde::Deserialize;

use super::{Shared, ask, logged_in_page, require_session, unexpected};
use crate::html::{Lang, Nav, escape, format_time};

/// At most this many entries on the page.
const PAGE_LIMIT: u32 = 500;
/// Periods offered as links, in days.
const PERIODS: [u32; 3] = [1, 7, 30];
const SECONDS_PER_DAY: i64 = 24 * 60 * 60;

/// Query parameters of the page.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ChangesQuery {
    /// Only the last `days` days; otherwise everything that is kept.
    days: Option<u32>,
}

/// The texts in `protonsync_client` take its language type.
const fn client_lang(lang: Lang) -> protonsync_client::Lang {
    match lang {
        Lang::De => protonsync_client::Lang::De,
        Lang::En => protonsync_client::Lang::En,
    }
}

pub(crate) async fn changes<B: crate::Backend>(
    State(state): Shared<B>,
    visitor: crate::visitor::Visitor,
    Query(query): Query<ChangesQuery>,
) -> Response {
    let session = match require_session(&visitor) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let lang = visitor.lang;
    let since = query.days.map(|days| {
        jiff::Timestamp::now()
            .as_second()
            .saturating_sub(i64::from(days) * SECONDS_PER_DAY)
    });
    let request = Request::Changes {
        since,
        pair: None,
        limit: PAGE_LIMIT,
    };
    let (changes, keep_days) = match ask(&state, &visitor, session, request).await {
        Ok(ApiResponse::Changes { changes, keep_days }) => (changes, keep_days),
        Ok(other) => return unexpected(&visitor, session, &other),
        Err(response) => return response,
    };
    let body = page_body(lang, &state.zone, &changes, keep_days, query.days);
    logged_in_page(
        &visitor,
        session,
        Nav::Changes,
        lang.pick("Änderungen", "Changes"),
        &body,
    )
}

fn page_body(
    lang: Lang,
    zone: &jiff::tz::TimeZone,
    changes: &[ChangeInfo],
    keep_days: u32,
    days: Option<u32>,
) -> String {
    if keep_days == 0 {
        return format!(
            "<p>{}</p>",
            lang.pick(
                "Das Änderungsprotokoll ist ausgeschaltet (<code>change_log_days = 0</code> in <code>[general]</code>).",
                "The change log is switched off (<code>change_log_days = 0</code> in <code>[general]</code>).",
            )
        );
    }
    let mut body = format!(
        "<p>{} {keep_days} {}</p><p>",
        lang.pick(
            "Jede Änderung, die ein Abgleich gemacht hat, neueste zuerst. Aufbewahrt:",
            "Every change a sync made, newest first. Kept for:"
        ),
        lang.pick("Tage.", "days.")
    );
    for period in PERIODS {
        let label = format!("{period} {}", lang.pick("Tage", "days"));
        period_link(
            &mut body,
            days == Some(period),
            &format!("?days={period}"),
            &label,
        );
    }
    period_link(
        &mut body,
        days.is_none(),
        "/changes",
        lang.pick("alle", "all"),
    );
    body.push_str("</p>");
    if changes.is_empty() {
        let _ = write!(
            body,
            "<p>{}</p>",
            lang.pick("Keine Änderungen protokolliert.", "No changes logged.")
        );
        return body;
    }
    let _ = write!(
        body,
        "<table><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr>",
        lang.pick("Zeit", "Time"),
        lang.pick("Sync-Paar", "Sync pair"),
        lang.pick("Geändert", "Changed"),
        lang.pick("Pfad", "Path"),
        lang.pick("Was", "What"),
    );
    for change in changes {
        let _ = write!(
            body,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}{}</td><td>{}</td></tr>",
            format_time(change.at, zone),
            escape(&change.pair),
            match change.side {
                ChangedSideInfo::Remote => "Proton",
                ChangedSideInfo::Local => lang.pick("lokal", "local"),
            },
            escape(&change.path),
            if change.is_folder { "/" } else { "" },
            escape(describe(client_lang(lang), change)),
        );
    }
    body.push_str("</table>");
    if changes.len() >= usize::try_from(PAGE_LIMIT).unwrap_or(usize::MAX) {
        let _ = write!(
            body,
            "<p>{} {PAGE_LIMIT}; {}</p>",
            lang.pick("Gezeigt werden die neuesten", "Showing the newest"),
            lang.pick(
                "alle zeigt <code>protonsync changes</code>.",
                "<code>protonsync changes</code> shows all."
            )
        );
    }
    body
}

fn period_link(body: &mut String, current: bool, href: &str, label: &str) {
    if current {
        let _ = write!(body, "<strong>{}</strong> ", escape(label));
    } else {
        let _ = write!(body, "<a href=\"{}\">{}</a> ", escape(href), escape(label));
    }
}
