//! Who is viewing: the gate is asked about every request, open page or
//! not, and each request reads the source `Sources` gives its viewer.

use crate::testing::start_time;
use crate::*;
use axum::body::Body;
use axum::http::Request as HttpRequest;
use futures::future::BoxFuture;
use kkt_clock::testing::FixedClock;
use kkt_config::PreviewSettings;
use kkt_entropy::testing::SeededEntropy;
use kkt_shutdown::Shutdown;
use std::sync::Mutex;
use tower::ServiceExt;

/// A gate whose viewer is named by the session cookie, `default` without
/// one; or, with a `refusal`, one that gives that answer to everyone. It
/// counts the requests it is asked about.
struct Named {
    sign_in: bool,
    refusal: Option<Admit<String>>,
    asked: Mutex<usize>,
    /// The sessions signed out, in order.
    signed_out: Mutex<Vec<String>>,
}

impl Named {
    fn open() -> Named {
        Named {
            sign_in: false,
            refusal: None,
            asked: Mutex::new(0),
            signed_out: Mutex::new(Vec::new()),
        }
    }
}

impl Gate for Named {
    type Viewer = String;

    fn sign_in_required(&self) -> bool {
        self.sign_in
    }

    fn admits<'a>(&'a self, session_id: Option<&'a str>) -> BoxFuture<'a, Admit<String>> {
        *self.asked.lock().expect("count") += 1;
        let admit = match &self.refusal {
            Some(refusal) => refusal.clone(),
            None => Admit::Yes(session_id.unwrap_or("default").to_string()),
        };
        Box::pin(std::future::ready(admit))
    }

    fn sign_in(&self, _key: &str) -> SignIn {
        SignIn::Refused("no sign-in here".into())
    }

    fn sign_out(&self, session_id: &str) {
        self.signed_out
            .lock()
            .expect("signed out")
            .push(session_id.to_string());
    }

    fn form_token(&self, _session_id: Option<&str>) -> Option<String> {
        Some("token".into())
    }

    fn form_token_matches(&self, _session_id: Option<&str>, presented: &str) -> bool {
        presented == "token"
    }
}

/// A preview world for each viewer name, made on first use and kept, with
/// a record of the viewers asked for.
struct PerViewer {
    worlds: Mutex<Vec<(String, Arc<Preview>)>>,
    asked: Mutex<Vec<String>>,
}

impl PerViewer {
    fn world(&self, viewer: &str) -> Arc<Preview> {
        let mut worlds = self.worlds.lock().expect("worlds");
        if let Some((_, world)) = worlds.iter().find(|(name, _)| name == viewer) {
            return world.clone();
        }
        let seed = worlds.len() as u64 + 1;
        let world = Arc::new(
            Preview::new(
                Arc::new(FixedClock::new(start_time())),
                Arc::new(SeededEntropy::new(seed)),
                PreviewSettings::DEFAULT,
                &Shutdown::new(),
            )
            .expect("the preview's constants are valid"),
        );
        worlds.push((viewer.to_string(), world.clone()));
        world
    }
}

impl Sources for PerViewer {
    type Viewer = String;

    fn source(&self, viewer: &String) -> Arc<dyn Source> {
        self.asked.lock().expect("asked").push(viewer.clone());
        self.world(viewer)
    }
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

/// The ID of each channel `source`'s front page lists.
async fn channel_ids(source: &Preview) -> Vec<String> {
    let overview = source.overview().await.expect("overview");
    overview
        .channels
        .into_iter()
        .map(|c| c.channel.id)
        .collect()
}

#[tokio::test]
async fn each_request_reads_the_source_of_the_viewer_the_gate_names() {
    let gate = Arc::new(Named::open());
    let sources = Arc::new(PerViewer {
        worlds: Mutex::new(Vec::new()),
        asked: Mutex::new(Vec::new()),
    });
    let site = router(
        sources.clone(),
        AllowedHosts::new(&[]).expect("loopback hosts"),
        gate.clone(),
    );

    let (status, default_page) = get_as(&site, "/", None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, alice_page) = get_as(&site, "/", Some("alice")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = get_as(&site, "/history", Some("alice")).await;
    assert_eq!(status, StatusCode::OK);

    // The gate was asked about every request, though the page is open, and
    // each request read its own viewer's source.
    assert_eq!(*gate.asked.lock().expect("count"), 3);
    assert_eq!(
        *sources.asked.lock().expect("asked"),
        ["default", "alice", "alice"]
    );
    let default_ids = channel_ids(&sources.world("default")).await;
    let alice_ids = channel_ids(&sources.world("alice")).await;
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
/// would help; otherwise `403` with the gate's reason. No source is made.
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
        let sources = Arc::new(PerViewer {
            worlds: Mutex::new(Vec::new()),
            asked: Mutex::new(Vec::new()),
        });
        let site = router(
            sources.clone(),
            AllowedHosts::new(&[]).expect("loopback hosts"),
            Arc::new(Named {
                sign_in,
                refusal: Some(refusal),
                asked: Mutex::new(0),
                signed_out: Mutex::new(Vec::new()),
            }),
        );
        for (path, expected) in [("/", page), ("/events", feed)] {
            let (status, body) = get_as(&site, path, Some("someone")).await;
            assert_eq!(status, expected.0, "{case}: {path}");
            if let Some(reason) = expected.1 {
                assert_eq!(body, reason, "{case}: {path}");
            }
        }
        assert!(sources.asked.lock().expect("asked").is_empty(), "{case}");
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
        let gate = Arc::new(Named {
            sign_in: true,
            refusal,
            asked: Mutex::new(0),
            signed_out: Mutex::new(Vec::new()),
        });
        let site = router(
            Arc::new(PerViewer {
                worlds: Mutex::new(Vec::new()),
                asked: Mutex::new(Vec::new()),
            }),
            AllowedHosts::new(&[]).expect("loopback hosts"),
            gate.clone(),
        );
        let request = HttpRequest::builder()
            .method("POST")
            .uri("/logout")
            .header("Host", "localhost")
            .header("Origin", "http://localhost")
            .header("Cookie", "kkt_session=someone")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(Body::from("form_token=token"))
            .expect("request");
        let response = site.oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::SEE_OTHER, "{case}");
        let signed_out = gate.signed_out.lock().expect("signed out").clone();
        let expected: Vec<String> = match ended {
            true => vec!["someone".into()],
            false => Vec::new(),
        };
        assert_eq!(signed_out, expected, "{case}");
    }
}
