//! Read-only workboard presentation. The server and preview share this mapper.
use super::source::{WorkDiagnostic, WorkItem, WorkboardView};
use crate::config::*;
use crate::workboard::Board;

pub(super) fn view(
    channel_id: String,
    channel_name: String,
    hidden: bool,
    now: i64,
    board: Option<Board>,
    unavailable: Option<String>,
) -> WorkboardView {
    let mut v = WorkboardView {
        channel_id,
        channel_name,
        hidden,
        now,
        high_watermark: 0,
        items: Vec::new(),
        diagnostics: Vec::new(),
        omitted_diagnostics: 0,
        unavailable,
        coverage: "Automatic verification not configured".into(),
    };
    if let Some(b) = board {
        v.high_watermark = b.last_message_id;
        v.omitted_diagnostics = b.omitted_diagnostics;
        v.diagnostics = b
            .diagnostics
            .into_iter()
            .map(|d| WorkDiagnostic {
                message_id: d.message_id,
                explanation: d.explanation,
            })
            .collect();
        let blocked_counts: std::collections::BTreeMap<_, _> = b
            .items
            .iter()
            .map(|(id, item)| {
                let count = item
                    .affected_tasks()
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .filter(|task| b.items.get(*task).is_some_and(|t| t.status == "blocked"))
                    .count();
                (id.clone(), count)
            })
            .collect();
        let mut entries: Vec<_> = b.items.into_values().collect();
        entries.sort_by_key(|i| {
            let group = if i.kind.is_request() && i.status_at(now) == "pending" {
                0
            } else if i.kind.is_external() {
                3
            } else if matches!(i.status.as_str(), "in_progress" | "blocked") {
                1
            } else if i.status == "queued" {
                2
            } else {
                4
            };
            (
                group,
                i.priority(),
                std::cmp::Reverse(blocked_counts.get(&i.id).copied().unwrap_or(0)),
                i.source_message_id,
            )
        });
        for i in entries {
            if let Some((owner, cadence)) = i.verifier()
                && !matches!(i.status.as_str(), "done" | "cancelled")
            {
                let last = i.verified_at.unwrap_or(i.updated_at);
                v.coverage = format!(
                    "Verifier: {owner}; checks every {cadence} seconds{}",
                    if now.saturating_sub(last) > cadence {
                        " — overdue"
                    } else {
                        ""
                    }
                );
            }
            let external = i.kind.is_external();
            let threshold = if external {
                WORKBOARD_EXTERNAL_STALE_SECONDS
            } else {
                WORKBOARD_ACTIVE_STALE_SECONDS
            };
            let stale = i
                .verified_at
                .is_none_or(|at| now.saturating_sub(at) > threshold);
            let freshness = match i.verified_at {
                Some(at) => format!(
                    "Verified at {}{}",
                    chrono::DateTime::from_timestamp(at, 0)
                        .map(crate::clock::to_text)
                        .unwrap_or_else(|| at.to_string()),
                    if stale { " — stale" } else { "" }
                ),
                None => "Not verified".into(),
            };
            let status = i.status_at(now).to_string();
            let group = if i.kind.is_request() && status == "pending" {
                "Needs you"
            } else if i.kind.is_external() {
                "PRs and issues"
            } else if status == "queued" {
                "Queued"
            } else if matches!(status.as_str(), "in_progress" | "blocked") {
                "Active work"
            } else {
                "History"
            };
            v.items.push(WorkItem {
                id: i.id.clone(),
                kind: i.kind.label().into(),
                title: i.title.clone(),
                status,
                owner: i.owner().into(),
                creator: i.creator.clone(),
                editor: i.editor.clone(),
                revision: i.revision,
                source_message_id: i.source_message_id,
                group: group.into(),
                stale,
                freshness,
                details: i.details(),
                external_url: i.external_url().map(str::to_string),
                response_url: i.response_url().map(str::to_string),
                resolution_url: i.resolution.as_ref().and_then(|r| r.evidence_url.clone()),
                evidence_ids: i.evidence_ids(),
            });
        }
    }
    v
}

use super::source::WorkboardPolicy;
use super::*;

#[derive(Deserialize, Default)]
pub(super) struct Params {
    channels: Option<String>,
    hidden: Option<String>,
}
struct ChannelChoice {
    name: String,
    selected: bool,
    url: String,
}
#[derive(Template)]
#[template(path = "workboard.html")]
struct WorkPage {
    boards: Vec<WorkboardView>,
    choices: Vec<ChannelChoice>,
    content_url: String,
    policy: WorkboardPolicy,
    sign_out: bool,
    form_token: String,
}
#[derive(Template)]
#[template(path = "workboard_content.html")]
struct WorkContent {
    boards: Vec<WorkboardView>,
}

pub(super) async fn channel_work(
    State(pages): State<Pages>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    render(
        pages,
        vec![id.clone()],
        true,
        format!("/channels/{}/work/content", url_path(&id)),
        headers,
        false,
    )
    .await
}
pub(super) async fn channel_content(
    State(pages): State<Pages>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    render(pages, vec![id], true, String::new(), headers, true).await
}
pub(super) async fn aggregate(
    State(pages): State<Pages>,
    Query(params): Query<Params>,
    headers: HeaderMap,
) -> Response {
    selected(pages, params, headers, false).await
}
pub(super) async fn aggregate_content(
    State(pages): State<Pages>,
    Query(params): Query<Params>,
    headers: HeaderMap,
) -> Response {
    selected(pages, params, headers, true).await
}
async fn selected(pages: Pages, params: Params, headers: HeaderMap, content: bool) -> Response {
    let raw = params.channels.unwrap_or_default();
    if raw.len() > WORKBOARD_MAX_CHANNELS.saturating_mul(WORKBOARD_MAX_ID_BYTES + 1) {
        return pages.error(
            SourceError::Invalid("Channel selection is too long".into()),
            &headers,
        );
    }
    let mut ids: Vec<_> = raw
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if ids
        .iter()
        .any(|s| !s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'))
    {
        return pages.error(SourceError::Invalid("Invalid channel ID".into()), &headers);
    }
    ids.sort();
    ids.dedup();
    let hidden = match params.hidden.as_deref() {
        Some("1") => true,
        None | Some("0") => false,
        _ => {
            return pages.error(
                SourceError::Invalid("hidden must be 0 or 1".into()),
                &headers,
            );
        }
    };
    let url = format!(
        "/work/content?channels={}&hidden={}",
        ids.join(","),
        u8::from(hidden)
    );
    render(pages, ids, hidden, url, headers, content).await
}
async fn render(
    pages: Pages,
    ids: Vec<String>,
    hidden: bool,
    content_url: String,
    headers: HeaderMap,
    content: bool,
) -> Response {
    let boards = match pages.source.workboards(ids.clone(), hidden).await {
        Ok(b) => b,
        Err(e) => return pages.error(e, &headers),
    };
    let response = if content {
        page(WorkContent { boards })
    } else {
        let overview = match pages.source.overview().await {
            Ok(v) => v,
            Err(e) => return pages.error(e, &headers),
        };
        let mut channels = overview.channels;
        if hidden {
            match pages.source.hidden_channels().await {
                Ok(v) => channels.extend(v),
                Err(e) => return pages.error(e, &headers),
            }
        }
        let mut name_counts = std::collections::BTreeMap::new();
        for channel in &channels {
            *name_counts
                .entry(channel.channel.name.as_str())
                .or_insert(0usize) += 1;
        }
        let duplicate_names: std::collections::BTreeSet<_> = name_counts
            .into_iter()
            .filter(|(_, count)| *count > 1)
            .map(|(name, _)| name.to_string())
            .collect();
        let choices = channels
            .into_iter()
            .map(|c| {
                let name = if duplicate_names.contains(&c.channel.name) {
                    format!("{} · {}", c.channel.name, c.channel.id)
                } else {
                    c.channel.name
                };
                let selected = ids.contains(&c.channel.id);
                let mut next = ids.clone();
                if selected {
                    next.retain(|id| id != &c.channel.id);
                } else {
                    next.push(c.channel.id);
                }
                next.sort();
                next.dedup();
                ChannelChoice {
                    name,
                    selected,
                    url: format!(
                        "/work?channels={}&hidden={}",
                        next.join(","),
                        u8::from(hidden)
                    ),
                }
            })
            .collect();
        page(WorkPage {
            boards,
            choices,
            content_url,
            policy: pages.source.workboard_policy(),
            sign_out: pages.sign_out,
            form_token: pages.form_token(&headers),
        })
    };
    let mut response = response;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}
