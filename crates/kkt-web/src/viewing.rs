//! Who is viewing each request. [`admit`] asks the [`Gate`] about every
//! request to a page or the live feed and puts the [`Source`] that
//! [`Sources`] gives the admitted viewer on the request, where [`Pages`]
//! finds it. A viewer who has not signed in goes to the login form, or gets
//! `401` on the live feed; any other refusal gets `403` with the gate's
//! reason, since there is nothing to sign in to. The viewer's type is the gate's own: only this
//! file sees it, so the pages and forms are written once for any gate.
//!
//! [`Forms`] is the rest of the gate, which needs no viewer: sign-in,
//! sign-out and form tokens.
//!
//! [`Pages`]: super::Pages

use super::sign_in::{EVENTS_PATH, LOGIN_PATH, session_cookie};
use super::source::{Admit, Gate, SignIn, Source, Sources, Viewer};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use futures::future::BoxFuture;
use std::sync::Arc;

/// The gate and the sources for viewers of type `V`.
pub(crate) struct Viewing<V> {
    pub gate: Arc<dyn Gate<Viewer = V>>,
    pub sources: Arc<dyn Sources<Viewer = V>>,
}

/// The source the admitted viewer of this request reads, put on the
/// request by [`admit`].
#[derive(Clone)]
pub(crate) struct ViewerSource(pub Arc<dyn Source>);

/// Lets a request through only once the gate admits its viewer, with that
/// viewer's source on the request. A viewer who must sign in is sent to the
/// login form, or gets `401` on the live feed. A viewer refused otherwise
/// gets `403` and the reason: one the gate refused though they were let
/// in, or any refusal where there is no sign-in.
pub(crate) async fn admit<V: Viewer>(
    State(viewing): State<Arc<Viewing<V>>>,
    mut request: Request,
    next: Next,
) -> Response {
    let session_id = session_cookie(request.headers());
    match viewing.gate.admits(session_id.as_deref()).await {
        Admit::Yes(viewer) => {
            let source = viewing.sources.source(&viewer);
            request.extensions_mut().insert(ViewerSource(source));
            next.run(request).await
        }
        Admit::No(reason) if !viewing.gate.sign_in_required() => {
            (StatusCode::FORBIDDEN, reason).into_response()
        }
        Admit::No(reason) if request.uri().path() == EVENTS_PATH => {
            (StatusCode::UNAUTHORIZED, reason).into_response()
        }
        Admit::No(_) => Redirect::to(LOGIN_PATH).into_response(),
        Admit::Refused(reason) => (StatusCode::FORBIDDEN, reason).into_response(),
    }
}

/// The part of a [`Gate`] that needs no viewer, for the pages, the forms
/// and signing in and out.
pub(crate) trait Forms: Send + Sync {
    /// Whether the viewer with this session (if any) is admitted.
    fn admitted<'a>(&'a self, session_id: Option<&'a str>) -> BoxFuture<'a, bool>;

    fn sign_in(&self, key: &str) -> SignIn;

    fn sign_out(&self, session_id: &str);

    fn form_token(&self, session_id: Option<&str>) -> Option<String>;

    fn form_token_matches(&self, session_id: Option<&str>, presented: &str) -> bool;
}

/// A gate's [`Forms`].
pub(crate) struct GateForms<V>(pub Arc<dyn Gate<Viewer = V>>);

impl<V: Viewer> Forms for GateForms<V> {
    fn admitted<'a>(&'a self, session_id: Option<&'a str>) -> BoxFuture<'a, bool> {
        Box::pin(async move { matches!(self.0.admits(session_id).await, Admit::Yes(_)) })
    }

    fn sign_in(&self, key: &str) -> SignIn {
        self.0.sign_in(key)
    }

    fn sign_out(&self, session_id: &str) {
        self.0.sign_out(session_id)
    }

    fn form_token(&self, session_id: Option<&str>) -> Option<String> {
        self.0.form_token(session_id)
    }

    fn form_token_matches(&self, session_id: Option<&str>, presented: &str) -> bool {
        self.0.form_token_matches(session_id, presented)
    }
}
