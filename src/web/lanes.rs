//! The Lanes pages: the list with a form for a new lane, and one lane with
//! its editor, its history, and a button to retire it.

use super::change::{Change, NoFields};
use super::source::{LaneLimits, LaneRetired, LaneSave, LaneSummary, LaneVersion};
use super::{Pages, filters, page, url_path};
use askama::Template;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

pub(super) const LANES_PATH: &str = "/lanes";

#[derive(Template)]
#[template(path = "lanes.html")]
struct LanesPage {
    lanes: Vec<LaneSummary>,
    limits: LaneLimits,
    sign_out: bool,
    form_token: String,
}

#[derive(Template)]
#[template(path = "lane.html")]
struct LanePage {
    name: String,
    created_at: String,
    retired: Option<LaneRetired>,
    current: LaneVersion,
    /// Newest first.
    history: Vec<LaneVersion>,
    limits: LaneLimits,
    sign_out: bool,
    form_token: String,
}

fn lane_path(name: &str) -> String {
    format!("{LANES_PATH}/{}", url_path(name))
}

pub(super) async fn lanes(State(pages): State<Pages>, headers: HeaderMap) -> Response {
    match pages.source.lanes().await {
        Ok(view) => page(LanesPage {
            lanes: view.lanes,
            limits: view.limits,
            sign_out: pages.sign_out,
            form_token: pages.form_token(&headers),
        }),
        Err(e) => pages.error(e, &headers),
    }
}

pub(super) async fn lane(
    State(pages): State<Pages>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    match pages.source.lane(name).await {
        Ok(view) => match view.history.first().cloned() {
            Some(current) => page(LanePage {
                name: view.name,
                created_at: view.created_at,
                retired: view.retired,
                current,
                history: view.history,
                limits: view.limits,
                sign_out: pages.sign_out,
                form_token: pages.form_token(&headers),
            }),
            None => pages.error(
                super::SourceError::Failed("the lane has no versions".into()),
                &headers,
            ),
        },
        Err(e) => pages.error(e, &headers),
    }
}

#[derive(Deserialize)]
pub(super) struct NewLaneForm {
    name: String,
    description: String,
    text: String,
}

/// The human creates a lane, then sees it.
pub(super) async fn create_lane(
    State(pages): State<Pages>,
    headers: HeaderMap,
    Change(form): Change<NewLaneForm>,
) -> Response {
    let name = form.name.trim().to_string();
    let save = LaneSave {
        name: name.clone(),
        description: form.description,
        text: form.text,
    };
    match pages.source.save_lane(save, true).await {
        Ok(()) => Redirect::to(&lane_path(&name)).into_response(),
        Err(e) => pages.error(e, &headers),
    }
}

#[derive(Deserialize)]
pub(super) struct LaneForm {
    description: String,
    text: String,
}

/// The human saves a lane's next version, then sees the lane again.
pub(super) async fn save_lane(
    State(pages): State<Pages>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Change(form): Change<LaneForm>,
) -> Response {
    let save = LaneSave {
        name: name.clone(),
        description: form.description,
        text: form.text,
    };
    match pages.source.save_lane(save, false).await {
        Ok(()) => Redirect::to(&lane_path(&name)).into_response(),
        Err(e) => pages.error(e, &headers),
    }
}

/// The human retires a lane, then sees it marked retired.
pub(super) async fn retire_lane(
    State(pages): State<Pages>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Change(NoFields {}): Change<NoFields>,
) -> Response {
    match pages.source.retire_lane(name.clone()).await {
        Ok(()) => Redirect::to(&lane_path(&name)).into_response(),
        Err(e) => pages.error(e, &headers),
    }
}
