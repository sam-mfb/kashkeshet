//! Logging in to the web page when a web key is set.

mod common;

use futures::future::BoxFuture;
use kkt_clock::{Clock, Span, Time};
use reqwest::StatusCode;
use reqwest::redirect::Policy;
use std::sync::Arc;
use tokio::sync::watch;

const AGENT_KEY: &str = "agent-key-0123456789abcdef0123456789abcdef";
const KEY: &str = "0123456789abcdef0123456789abcdef";

/// A clock the test moves by hand.
struct TestClock(watch::Sender<Time>);

impl TestClock {
    fn advance(&self, by: Span) {
        self.0.send_modify(|now| *now += by);
    }
}

impl Clock for TestClock {
    fn now(&self) -> Time {
        *self.0.borrow()
    }

    fn sleep_until(&self, deadline: Time) -> BoxFuture<'_, ()> {
        let mut now = self.0.subscribe();
        Box::pin(async move {
            now.wait_for(|t| *t >= deadline)
                .await
                .expect("clock outlives its sleeps");
        })
    }
}

struct Site {
    server: common::Server,
    http: reqwest::Client,
}

/// How a test's server is started.
enum Start {
    /// As `KKT_TEST_PROGRAM` says, with the web key.
    Keyed,
    /// In this process, with the web key and a clock the test moves.
    KeyedWithClock(Arc<TestClock>),
    /// In this process, with an access key but no web key: settings only
    /// this process can have, since `kkt serve` makes a web key.
    OpenWebPage,
}

async fn start(how: Start) -> Site {
    let mut config = common::test_config();
    config.access_key = Some(AGENT_KEY.into());
    config.web_key = match how {
        Start::OpenWebPage => None,
        Start::Keyed | Start::KeyedWithClock(_) => Some(KEY.into()),
    };
    config.allowed_hosts = vec!["host.docker.internal".into()];
    let server = match how {
        Start::Keyed => common::start_with(config).await,
        Start::KeyedWithClock(clock) => common::start_with_clock(config, clock).await,
        Start::OpenWebPage => common::start_in_process(config).await,
    };
    let http = reqwest::Client::builder()
        .redirect(Policy::none())
        .build()
        .expect("client");
    Site { server, http }
}

async fn start_keyed() -> Site {
    start(Start::Keyed).await
}

/// A clock at a fixed start that the test moves by hand.
fn test_clock() -> Arc<TestClock> {
    Arc::new(TestClock(watch::Sender::new(
        Time::from_text("2026-01-01T00:00:00Z").unwrap(),
    )))
}

impl Site {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.server.base)
    }

    async fn get(&self, path: &str, cookie: Option<&str>) -> reqwest::Response {
        let mut request = self.http.get(self.url(path));
        if let Some(c) = cookie {
            request = request.header("cookie", c);
        }
        request.send().await.expect("get")
    }

    async fn post_form(
        &self,
        path: &str,
        body: &str,
        cookie: Option<&str>,
        origin: Option<&str>,
    ) -> reqwest::Response {
        let mut request = self
            .http
            .post(self.url(path))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body.to_string());
        if let Some(c) = cookie {
            request = request.header("cookie", c);
        }
        if let Some(o) = origin {
            request = request.header("origin", o);
        }
        request.send().await.expect("post")
    }

    fn own_origin(&self) -> String {
        self.server.base.clone()
    }

    /// The form token on the front page, as the viewer with `cookie` sees it.
    async fn form_token(&self, cookie: Option<&str>) -> String {
        let page = self.get("/", cookie).await.text().await.expect("page");
        form_token_in(&page).expect("a form token on the page")
    }

    /// Logs in with the right key and returns the cookie to send back.
    async fn log_in(&self) -> String {
        let response = self
            .post_form(
                "/login",
                &format!("key={KEY}"),
                None,
                Some(&self.own_origin()),
            )
            .await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(location(&response), Some("/"));
        let set = header(&response, "set-cookie").expect("set-cookie");
        set.split(';').next().expect("name=value").to_string()
    }
}

/// The value of the first `form_token` field in `page`.
fn form_token_in(page: &str) -> Option<String> {
    let (_, rest) = page.split_once(r#"name="form_token" value=""#)?;
    rest.split_once('"').map(|(token, _)| token.to_string())
}

fn header<'a>(response: &'a reqwest::Response, name: &str) -> Option<&'a str> {
    response.headers().get(name).and_then(|v| v.to_str().ok())
}

fn location(response: &reqwest::Response) -> Option<&str> {
    header(response, "location")
}

fn assert_sent_to_login(response: &reqwest::Response) {
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(response), Some("/login"));
}

#[tokio::test]
async fn without_a_key_nothing_changes() {
    let site = start(Start::OpenWebPage).await;
    let page = site.get("/", None).await;
    assert_eq!(page.status(), StatusCode::OK);
    assert!(!page.text().await.expect("body").contains("Log out"));
    assert_eq!(site.get("/events", None).await.status(), StatusCode::OK);
    assert_eq!(
        site.get("/login", None).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn pages_without_a_session_go_to_login() {
    let site = start_keyed().await;
    assert_sent_to_login(&site.get("/", None).await);
    assert_sent_to_login(&site.get("/channels/ch_none", None).await);
    assert_sent_to_login(&site.get("/agents/nobody", None).await);
    assert_sent_to_login(&site.get("/", Some("kkt_session=made-up")).await);
    assert_eq!(
        site.get("/events", None).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let form = site.get("/login", None).await;
    assert_eq!(form.status(), StatusCode::OK);
    assert!(form.text().await.expect("body").contains("Web key"));
}

#[tokio::test]
async fn a_wrong_key_is_refused() {
    let site = start_keyed().await;
    let response = site
        .post_form(
            "/login",
            "key=0123456789abcdef0123456789abcdeX",
            None,
            Some(&site.own_origin()),
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(header(&response, "set-cookie").is_none());
    assert!(response.text().await.expect("body").contains("wrong"));
}

#[tokio::test]
async fn the_right_key_gives_a_working_cookie() {
    let site = start_keyed().await;
    let response = site
        .post_form(
            "/login",
            &format!("key={KEY}"),
            None,
            Some(&site.own_origin()),
        )
        .await;
    let set = header(&response, "set-cookie").expect("set-cookie");
    for attribute in ["HttpOnly", "SameSite=Strict", "Path=/", "Max-Age=43200"] {
        assert!(set.contains(attribute), "{set}");
    }
    // Sent to 127.0.0.1, a loopback name, so not `Secure`.
    assert!(!set.contains("Secure"), "{set}");
    assert!(!set.contains(KEY));

    let cookie = site.log_in().await;
    let page = site.get("/", Some(&cookie)).await;
    assert_eq!(page.status(), StatusCode::OK);
    assert!(page.text().await.expect("body").contains("Log out"));
    assert_eq!(
        site.get("/events", Some(&cookie)).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_session_expires() {
    let clock = test_clock();
    let site = start(Start::KeyedWithClock(clock.clone())).await;
    let cookie = site.log_in().await;
    clock.advance(Span::hours(12) - Span::seconds(1));
    assert_eq!(site.get("/", Some(&cookie)).await.status(), StatusCode::OK);
    clock.advance(Span::seconds(1));
    assert_sent_to_login(&site.get("/", Some(&cookie)).await);
    assert_eq!(
        site.get("/events", Some(&cookie)).await.status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn logging_out_ends_the_session() {
    let site = start_keyed().await;
    let cookie = site.log_in().await;
    // Without the page's form token, a live session is not ended.
    let response = site
        .post_form("/logout", "", Some(&cookie), Some(&site.own_origin()))
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(site.get("/", Some(&cookie)).await.status(), StatusCode::OK);

    let token = site.form_token(Some(&cookie)).await;
    let response = site
        .post_form(
            "/logout",
            &format!("form_token={token}"),
            Some(&cookie),
            Some(&site.own_origin()),
        )
        .await;
    assert_sent_to_login(&response);
    let cleared = header(&response, "set-cookie").expect("set-cookie");
    assert!(cleared.starts_with("kkt_session=;"), "{cleared}");
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
    assert_sent_to_login(&site.get("/", Some(&cookie)).await);
}

#[tokio::test]
async fn changes_need_the_sessions_form_token() {
    let site = start_keyed().await;
    let cookie = site.log_in().await;
    let other = site.log_in().await;
    let token = site.form_token(Some(&cookie)).await;
    let others = site.form_token(Some(&other)).await;
    assert_ne!(token, others, "each session has its own token");
    let change = |body: String, cookie: Option<String>| {
        let site = &site;
        async move {
            site.post_form(
                "/channels/ch_missing0/purpose",
                &body,
                cookie.as_deref(),
                Some(&site.own_origin()),
            )
            .await
            .status()
        }
    };
    // No token, a wrong one, or another session's: refused before anything
    // is looked up.
    for body in [
        "purpose=x".to_string(),
        "purpose=x&form_token=guess".to_string(),
        format!("purpose=x&form_token={others}"),
    ] {
        assert_eq!(
            change(body.clone(), Some(cookie.clone())).await,
            StatusCode::FORBIDDEN,
            "{body}"
        );
    }
    // The right token gets through to the change, which finds no channel.
    assert_eq!(
        change(
            format!("purpose=x&form_token={token}"),
            Some(cookie.clone())
        )
        .await,
        StatusCode::NOT_FOUND
    );
    // Without a session, a change goes to the login form like any page.
    let response = site
        .post_form(
            "/channels/ch_missing0/purpose",
            &format!("purpose=x&form_token={token}"),
            None,
            Some(&site.own_origin()),
        )
        .await;
    assert_sent_to_login(&response);
    // A form from another site is refused even with the token.
    let response = site
        .post_form(
            "/channels/ch_missing0/purpose",
            &format!("purpose=x&form_token={token}"),
            Some(&cookie),
            Some("http://evil.example"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn forms_from_another_site_are_refused() {
    let site = start_keyed().await;
    let response = site
        .post_form(
            "/login",
            &format!("key={KEY}"),
            None,
            Some("http://evil.example"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(header(&response, "set-cookie").is_none());

    let cookie = site.log_in().await;
    let response = site
        .post_form("/logout", "", Some(&cookie), Some("http://evil.example"))
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(site.get("/", Some(&cookie)).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_web_session_does_not_open_mcp() {
    let site = start_keyed().await;
    let cookie = site.log_in().await;
    let response = site
        .http
        .post(site.url("/mcp"))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("cookie", cookie)
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#)
        .send()
        .await
        .expect("post /mcp");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_and_logout_check_the_host() {
    let site = start_keyed().await;
    let cookie = site.log_in().await;
    for (path, body, cookie) in [
        ("/login", format!("key={KEY}"), None),
        ("/logout", String::new(), Some(cookie.as_str())),
    ] {
        let mut request = site
            .http
            .post(site.url(path))
            .header("host", "evil.example")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body);
        if let Some(c) = cookie {
            request = request.header("cookie", c);
        }
        let response = request.send().await.expect("post");
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
        assert!(header(&response, "set-cookie").is_none(), "{path}");
    }
    let form = site
        .http
        .get(site.url("/login"))
        .header("host", "evil.example")
        .send()
        .await
        .expect("get");
    assert_eq!(form.status(), StatusCode::FORBIDDEN);
    assert_eq!(site.get("/", Some(&cookie)).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn cookies_are_secure_except_on_loopback_names() {
    let site = start_keyed().await;
    let port = site
        .server
        .base
        .rsplit(':')
        .next()
        .expect("port")
        .to_string();
    let host = format!("host.docker.internal:{port}");
    let origin = format!("http://{host}");
    let response = site
        .http
        .post(site.url("/login"))
        .header("host", &host)
        .header("origin", &origin)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("key={KEY}"))
        .send()
        .await
        .expect("login");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let set = header(&response, "set-cookie").expect("set-cookie");
    assert!(set.contains("; Secure;"), "{set}");
    let cookie = set.split(';').next().expect("name=value").to_string();

    let token = site.form_token(Some(&cookie)).await;
    let response = site
        .http
        .post(site.url("/logout"))
        .header("host", &host)
        .header("origin", &origin)
        .header("cookie", &cookie)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("form_token={token}"))
        .send()
        .await
        .expect("logout");
    let cleared = header(&response, "set-cookie").expect("set-cookie");
    assert!(cleared.contains("; Secure;"), "{cleared}");

    let response = site
        .post_form("/logout", "", None, Some(&site.own_origin()))
        .await;
    let cleared = header(&response, "set-cookie").expect("set-cookie");
    assert!(!cleared.contains("Secure"), "{cleared}");
}

#[tokio::test]
async fn the_access_key_cannot_log_in_and_the_web_key_cannot_call_mcp() {
    let site = start_keyed().await;
    let login = site
        .post_form(
            "/login",
            &format!("key={AGENT_KEY}"),
            None,
            Some(&site.own_origin()),
        )
        .await;
    assert!(!login.headers().contains_key("set-cookie"));
    assert_eq!(site.get("/", None).await.status(), StatusCode::SEE_OTHER);
    let mcp = site
        .http
        .post(site.url("/mcp"))
        .bearer_auth(KEY)
        .send()
        .await
        .unwrap();
    assert_eq!(mcp.status(), StatusCode::UNAUTHORIZED);
    let mcp = site
        .http
        .post(site.url("/mcp"))
        .bearer_auth(AGENT_KEY)
        .send()
        .await
        .unwrap();
    assert_ne!(mcp.status(), StatusCode::UNAUTHORIZED);
    let _cookie = site.log_in().await;
}
