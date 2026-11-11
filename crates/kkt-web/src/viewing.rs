//! Who is viewing each request. [`admit`] asks the [`Gate`] about every
//! request to a page or the live feed and puts the [`Source`] that
//! [`Sources`] gives the admitted viewer on the request, where [`Pages`]
//! finds it. A viewer who has not signed in goes to the login form, or gets
//! `401` on the live feed; any other refusal gets `403` with the gate's
//! reason, since there is nothing to sign in to. The viewer's type is the gate's own: only this
//! file sees it, so the pages and forms are written once for any gate.
//!
//! [`Forms`] is the rest, which needs no viewer: sign-in, sign-out and form
//! tokens, over the page's own [`Sessions`] and the gate's sign-in check.
//!
//! [`Pages`]: super::Pages

use super::sessions::{Held, Sessions};
use super::sign_in::{EVENTS_PATH, LOGIN_PATH, session_cookie};
use super::source::{Admit, Gate, SignIn, Source, Sources, Viewer};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use futures::future::BoxFuture;
use std::sync::Arc;
use std::time::Duration;

/// The gate, the sessions and the sources for viewers of type `V`, whose
/// sessions hold `S`.
pub(crate) struct Viewing<V, S> {
    pub gate: Arc<dyn Gate<Viewer = V, SignedIn = S>>,
    pub sessions: Arc<Sessions<S>>,
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
pub(crate) async fn admit<V: Viewer, S: Held>(
    State(viewing): State<Arc<Viewing<V, S>>>,
    mut request: Request,
    next: Next,
) -> Response {
    let session_id = session_cookie(request.headers());
    let session = viewing.sessions.session(session_id.as_deref());
    match viewing.gate.admits(session).await {
        Admit::Yes(viewer) => {
            let source = viewing.sources.source(&viewer);
            request.extensions_mut().insert(ViewerSource(source));
            next.run(request).await
        }
        Admit::No(reason) if !viewing.sessions.sign_in_required() => {
            (StatusCode::FORBIDDEN, reason).into_response()
        }
        Admit::No(reason) if request.uri().path() == EVENTS_PATH => {
            (StatusCode::UNAUTHORIZED, reason).into_response()
        }
        Admit::No(_) => Redirect::to(LOGIN_PATH).into_response(),
        Admit::Refused(reason) => (StatusCode::FORBIDDEN, reason).into_response(),
    }
}

/// What the pages, the forms and signing in and out need, with no viewer:
/// the gate's sign-in check over the page's sessions.
pub(crate) trait Forms: Send + Sync {
    /// Whether the viewer with this session (if any) was let in: admitted,
    /// or refused for who they are ([`Admit::Refused`]). Either way their
    /// session is live, and signing out ends it.
    fn let_in<'a>(&'a self, session_id: Option<&'a str>) -> BoxFuture<'a, bool>;

    /// Checks the key a viewer typed and, if the gate lets them in, starts
    /// a session: its ID and how long it lasts, or the gate's reason.
    fn sign_in(&self, key: &str) -> Result<(String, Duration), String>;

    /// Ends a session.
    fn sign_out(&self, session_id: &str);

    /// The token the page puts in its forms for the viewer with this
    /// session (if any). `None` when the viewer has no live session and
    /// needs one.
    fn form_token(&self, session_id: Option<&str>) -> Option<String>;

    /// Whether `presented` is that viewer's form token. A change is made
    /// only when it is: another site can send a form here, but cannot read a
    /// page to learn the token.
    fn form_token_matches(&self, session_id: Option<&str>, presented: &str) -> bool;
}

/// A gate's [`Forms`], over the page's sessions.
pub(crate) struct GateForms<V, S> {
    pub gate: Arc<dyn Gate<Viewer = V, SignedIn = S>>,
    pub sessions: Arc<Sessions<S>>,
}

impl<V: Viewer, S: Held> Forms for GateForms<V, S> {
    fn let_in<'a>(&'a self, session_id: Option<&'a str>) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            let session = self.sessions.session(session_id);
            matches!(
                self.gate.admits(session).await,
                Admit::Yes(_) | Admit::Refused(_)
            )
        })
    }

    fn sign_in(&self, key: &str) -> Result<(String, Duration), String> {
        match self.gate.sign_in(key) {
            SignIn::Admitted(holder) => Ok((self.sessions.start(holder), self.sessions.lifetime())),
            SignIn::Refused(reason) => Err(reason),
        }
    }

    fn sign_out(&self, session_id: &str) {
        self.sessions.end(session_id)
    }

    fn form_token(&self, session_id: Option<&str>) -> Option<String> {
        self.sessions.form_token(session_id)
    }

    fn form_token_matches(&self, session_id: Option<&str>, presented: &str) -> bool {
        self.sessions.form_token_matches(session_id, presented)
    }
}
