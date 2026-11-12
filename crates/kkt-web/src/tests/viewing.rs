//! Who is viewing: the gate is asked about every request, open page or
//! not, and each request reads the application in its viewer's swarm.

use crate::preview::PREVIEW_SWARM;
use crate::testing::start_time;
use crate::*;
use axum::body::Body;
use axum::http::Request as HttpRequest;
use futures::future::BoxFuture;
use kkt_api_agent::{Audit, Operator, Permissions, SwarmId};
use kkt_api_operator::OperatorApi;
use kkt_clock::testing::FixedClock;
use kkt_entropy::SeededEntropy;
use kkt_entropy::testing::SeqEntropy;
use kkt_shutdown::Shutdown;
use std::sync::Mutex;
use tower::ServiceExt;

/// A gate whose viewer's swarm is named by what the session holds,
/// `default` without a live one; or, with a `refusal`, one that gives that
/// answer to everyone. It counts the requests it is asked about.
struct Named {
    refusal: Option<Admit<Viewer>>,
    asked: Mutex<usize>,
}

impl Named {
    fn open() -> Named {
        Named {
            refusal: None,
            asked: Mutex::new(0),
        }
    }
}

/// The viewer who sees the swarm `name`.
fn viewer(name: &str) -> Viewer {
    Viewer {
        swarm: SwarmId::new(name),
        operator: Operator::new(Permissions::of(&[]), Audit::WebPage),
    }
}

impl Gate for Named {
    type SignedIn = String;

    fn admits(&self, session: Session<String>) -> BoxFuture<'_, Admit<Viewer>> {
        *self.asked.lock().expect("count") += 1;
        let admit = match (&self.refusal, session) {
            (Some(refusal), _) => refusal.clone(),
            (None, Session::Live(name)) => Admit::Yes(viewer(&name)),
            (None, Session::Absent | Session::Ended) => Admit::Yes(viewer("default")),
        };
        Box::pin(std::future::ready(admit))
    }

    fn sign_in(&self, _key: &str) -> SignIn<String> {
        SignIn::Refused("no sign-in here".into())
    }
}

/// The page's sessions, with `access`, timed by a fixed clock.
fn sessions(access: Access) -> Arc<Sessions<String>> {
    Arc::new(Sessions::new(
        Arc::new(FixedClock::new(start_time())),
        Arc::new(SeqEntropy::new(5)),
        Settings::DEFAULT.session_lifetime,
        access,
    ))
}

/// A preview, which makes a world for each swarm the first time it is
/// asked for.
fn preview() -> Arc<Preview> {
    Arc::new(
        Preview::new(
            Arc::new(FixedClock::new(start_time())),
            Arc::new(SeededEntropy::new(1)),
            PreviewSettings::DEFAULT,
            &Shutdown::new(),
        )
        .expect("the preview's constants are valid"),
    )
}

async fn get_as(site: &Router, path: &str, session: Option<&str>) -> (StatusCode, String) {
    let mut request = HttpRequest::builder().uri(path).header("Host", "localhost");
    if let Some(id) = session {
        request = request.header("Cookie", format!("kkt_session={id}"));
    }
    let request = request.body(Body::empty()).expect("request");
    let response = site.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The ID of each channel the front page of `swarm`'s world lists.
async fn channel_ids(preview: &Preview, swarm: &str) -> Vec<String> {
    let overview = preview
        .in_swarm(SwarmId::new(swarm))
        .overview(WEB_ACTIVITY_WINDOW, WEB_ACTIVITY_BUCKET)
        .await
        .expect("overview");
    overview
        .channels
        .into_iter()
        .map(|c| c.channel.id)
        .collect()
}

#[tokio::test]
async fn each_request_reads_the_swarm_of_the_viewer_the_gate_names() {
    let gate = Arc::new(Named::open());
    let sources = preview();
    let sessions = sessions(Access::Open);
    let alice = sessions.start("alice".to_string());
    let site = router(
        sources.clone() as Arc<dyn OperatorApi>,
        AllowedHosts::new(&[]).expect("loopback hosts"),
        gate.clone(),
        sessions,
        Arc::new(kkt_log::testing::CapturedLog::default()),
        Settings::DEFAULT,
    );

    let (status, default_page) = get_as(&site, "/", None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, alice_page) = get_as(&site, "/", Some(&alice)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = get_as(&site, "/history", Some(&alice)).await;
    assert_eq!(status, StatusCode::OK);

    // The gate was asked about every request, though the page is open, and
    // each request read its own viewer's swarm.
    assert_eq!(*gate.asked.lock().expect("count"), 3);
    assert_eq!(
        sources.swarms(),
        [PREVIEW_SWARM, "default", "alice"].map(SwarmId::new)
    );
    let default_ids = channel_ids(&sources, "default").await;
    let alice_ids = channel_ids(&sources, "alice").await;
    assert_ne!(default_ids, alice_ids, "the two worlds differ");
    for id in &default_ids {
        assert!(default_page.contains(id.as_str()), "{id} on default's page");
    }
    for id in alice_ids.iter().filter(|id| !default_ids.contains(id)) {
        assert!(alice_page.contains(id.as_str()), "{id} on alice's page");
        assert!(!default_page.contains(id.as_str()), "{id} not on default's");
    }
}

/// What a refused viewer gets, by whether the site has sign-in and how the
/// gate refused: sent to sign in only when there is sign-in and signing in
/// would help; otherwise `403` with the gate's reason. No swarm is read.
#[tokio::test]
async fn a_refused_viewer_sees_no_page_and_is_told_why_unless_signing_in_helps() {
    let no = || Admit::No("sign in first".to_string());
    let refused = || Admit::Refused("not for you".to_string());
    let forbidden = |reason: &str| (StatusCode::FORBIDDEN, Some(reason.to_string()));
    let cases = [
        (
            false,
            no(),
            forbidden("sign in first"),
            forbidden("sign in first"),
        ),
        (
            false,
            refused(),
            forbidden("not for you"),
            forbidden("not for you"),
        ),
        (
            true,
            no(),
            (StatusCode::SEE_OTHER, None),
            (StatusCode::UNAUTHORIZED, Some("sign in first".to_string())),
        ),
        (
            true,
            refused(),
            forbidden("not for you"),
            forbidden("not for you"),
        ),
    ];
    for (sign_in, refusal, page, feed) in cases {
        let case = format!("sign-in {sign_in}, {refusal:?}");
        let sources = preview();
        let site = router(
            sources.clone() as Arc<dyn OperatorApi>,
            AllowedHosts::new(&[]).expect("loopback hosts"),
            Arc::new(Named {
                refusal: Some(refusal),
                asked: Mutex::new(0),
            }),
            sessions(match sign_in {
                true => Access::SignIn,
                false => Access::Open,
            }),
            Arc::new(kkt_log::testing::CapturedLog::default()),
            Settings::DEFAULT,
        );
        for (path, expected) in [("/", page), ("/events", feed)] {
            let (status, body) = get_as(&site, path, Some("someone")).await;
            assert_eq!(status, expected.0, "{case}: {path}");
            if let Some(reason) = expected.1 {
                assert_eq!(body, reason, "{case}: {path}");
            }
        }
        assert_eq!(sources.swarms(), [SwarmId::new(PREVIEW_SWARM)], "{case}");
    }
}

/// Signing out ends the session of a viewer who was let in, though refused
/// for who they are, not only an admitted one's; one who was never let in
/// has only the cookie cleared.
#[tokio::test]
async fn signing_out_ends_the_session_of_any_viewer_let_in() {
    for (refusal, ended) in [
        (None, true),
        (Some(Admit::Refused("not for you".to_string())), true),
        (Some(Admit::No("sign in first".to_string())), false),
    ] {
        let case = format!("{refusal:?}");
        let sessions = sessions(Access::SignIn);
        let id = sessions.start("someone".to_string());
        let token = sessions
            .form_token(Some(&id))
            .expect("a live session's token");
        let site = router(
            preview(),
            AllowedHosts::new(&[]).expect("loopback hosts"),
            Arc::new(Named {
                refusal,
                asked: Mutex::new(0),
            }),
            sessions.clone(),
            Arc::new(kkt_log::testing::CapturedLog::default()),
            Settings::DEFAULT,
        );
        let request = HttpRequest::builder()
            .method("POST")
            .uri("/logout")
            .header("Host", "localhost")
            .header("Origin", "http://localhost")
            .header("Cookie", format!("kkt_session={id}"))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(Body::from(format!("form_token={token}")))
            .expect("request");
        let response = site.oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::SEE_OTHER, "{case}");
        let expected = match ended {
            true => Session::Ended,
            false => Session::Live("someone".to_string()),
        };
        assert_eq!(sessions.session(Some(&id)), expected, "{case}");
    }
}

#[tokio::test]
async fn host_refusals_reach_the_injected_log_before_the_gate_is_asked() {
    let gate = Arc::new(Named::open());
    let sources = preview();
    let log = Arc::new(kkt_log::testing::CapturedLog::default());
    let site = router(
        sources.clone() as Arc<dyn OperatorApi>,
        AllowedHosts::new(&[]).expect("loopback hosts"),
        gate.clone(),
        sessions(Access::Open),
        log.clone(),
        Settings::DEFAULT,
    );
    let request = HttpRequest::builder()
        .uri("/")
        .header("Host", "bad.example")
        .body(Body::empty())
        .expect("request");
    let response = site.oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(*gate.asked.lock().expect("gate count"), 0);
    assert_eq!(sources.swarms(), [SwarmId::new(PREVIEW_SWARM)]);
    assert_eq!(
        log.entries(),
        vec![kkt_log::testing::Entry {
            level: kkt_log::Level::Warn,
            target: "kkt_web",
            message: "refused a web request naming a host that is not allowed".into(),
            fields: vec![("host", "Some(\"bad.example\")".into())],
        }]
    );
}
