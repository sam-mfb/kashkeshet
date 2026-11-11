//! The Lanes pages: the list with a form for a new lane, and one lane with
//! its editor, its history, and a button to retire it.

use super::change::{Change, NoFields};
use super::{Pages, filters, page, url_path};
use askama::Template;
use axum::extract::Path;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use kkt_api_agent::{Lane, LaneRetired, LaneVersion, Limits};
use kkt_api_operator::LaneSave;
use kkt_names::MAX_LANE_NAME_CHARS;
use serde::Deserialize;

pub(super) const LANES_PATH: &str = "/lanes";

#[derive(Template)]
#[template(path = "lanes.html")]
struct LanesPage {
    /// In name order.
    lanes: Vec<Lane>,
    /// The application's limits, for the lane's description and text.
    limits: Limits,
    max_name_chars: usize,
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
    /// The application's limits, for the lane's description and text.
    limits: Limits,
    sign_out: bool,
    form_token: String,
}

fn lane_path(name: &str) -> String {
    format!("{LANES_PATH}/{}", url_path(name))
}

pub(super) async fn lanes(pages: Pages, headers: HeaderMap) -> Response {
    match pages.api.lanes(&pages.operator).await {
        Ok(lanes) => page(
            pages.log.as_ref(),
            LanesPage {
                lanes,
                limits: pages.api.limits(),
                max_name_chars: MAX_LANE_NAME_CHARS,
                sign_out: pages.sign_out,
                form_token: pages.form_token(&headers),
            },
        ),
        Err(e) => pages.error(e, &headers),
    }
}

pub(super) async fn lane(pages: Pages, Path(name): Path<String>, headers: HeaderMap) -> Response {
    match pages.api.lane_history(&pages.operator, name).await {
        Ok(h) => page(
            pages.log.as_ref(),
            LanePage {
                name: h.lane.name,
                created_at: h.lane.created_at,
                retired: h.lane.retired,
                current: h.lane.current,
                history: h.versions,
                limits: pages.api.limits(),
                sign_out: pages.sign_out,
                form_token: pages.form_token(&headers),
            },
        ),
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
    pages: Pages,
    headers: HeaderMap,
    Change(form): Change<NewLaneForm>,
) -> Response {
    let name = form.name.trim().to_string();
    let save = LaneSave {
        name: name.clone(),
        description: form.description,
        text: form.text,
    };
    match pages.api.create_lane(pages.actor(), save).await {
        Ok(_) => Redirect::to(&lane_path(&name)).into_response(),
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
    pages: Pages,
    Path(name): Path<String>,
    headers: HeaderMap,
    Change(form): Change<LaneForm>,
) -> Response {
    let save = LaneSave {
        name: name.clone(),
        description: form.description,
        text: form.text,
    };
    match pages.api.update_lane(pages.actor(), save).await {
        Ok(_) => Redirect::to(&lane_path(&name)).into_response(),
        Err(e) => pages.error(e, &headers),
    }
}

/// The human retires a lane, then sees it marked retired.
pub(super) async fn retire_lane(
    pages: Pages,
    Path(name): Path<String>,
    headers: HeaderMap,
    Change(NoFields {}): Change<NoFields>,
) -> Response {
    match pages.api.retire_lane(pages.actor(), name.clone()).await {
        Ok(_) => Redirect::to(&lane_path(&name)).into_response(),
        Err(e) => pages.error(e, &headers),
    }
}
