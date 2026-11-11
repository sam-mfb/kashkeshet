use super::*;
use serde_json::json;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

/// The address the tests serve: kkt's default.
const DEFAULT_BIND: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 7420));

struct Fake {
    replies: RefCell<VecDeque<Result<Output, String>>>,
    calls: RefCell<Vec<Vec<String>>>,
}

impl Fake {
    fn new(replies: Vec<Result<Output, String>>) -> Self {
        Self {
            replies: RefCell::new(replies.into()),
            calls: RefCell::new(Vec::new()),
        }
    }
}

impl Host for Fake {
    fn run(&self, args: &[String]) -> Result<Output, String> {
        self.calls.borrow_mut().push(args.to_vec());
        self.replies
            .borrow_mut()
            .pop_front()
            .expect("unexpected command")
    }
}

fn reply(value: Value) -> Result<Output, String> {
    Ok(Output {
        success: true,
        stdout: value.to_string(),
        stderr: String::new(),
    })
}

fn running() -> Result<Output, String> {
    reply(
        json!({"BackendState":"Running", "Self":{"DNSName":"my-mac-1.tail123.ts.net."}, "CertDomains":["my-mac-1.tail123.ts.net"]}),
    )
}

fn proxy(target: &str) -> Value {
    json!({"TCP":{"443":{"HTTPS":true}}, "Web":{"my-mac-1.tail123.ts.net:443":{"Handlers":{"/":{"Proxy":target}, "/other":{"Path":"/somewhere"}}}}})
}

#[test]
fn prepare_keeps_the_exact_node_name_without_changing_login() {
    let host = Fake::new(vec![running()]);
    assert_eq!(prepare(&host).unwrap().dns_name, "my-mac-1.tail123.ts.net");
    assert_eq!(*host.calls.borrow(), vec![vec!["status", "--json"]]);
}

#[test]
fn prepare_reports_login_and_https_actions_before_any_changes() {
    for (value, expected) in [
        (json!({"BackendState":"NeedsLogin"}), "log in"),
        (json!({"BackendState":"Stopped"}), "Stopped"),
        (
            json!({"BackendState":"Running","Self":{"DNSName":"node.tail.ts.net"},"CertDomains":null}),
            "Enable HTTPS",
        ),
        (
            json!({"BackendState":"Running","Self":{"DNSName":"node.tail.ts.net"},"CertDomains":["other.tail.ts.net"]}),
            "Enable HTTPS",
        ),
        (
            json!({"BackendState":"Running","Self":{"DNSName":"node/path.tail.ts.net"},"CertDomains":[]}),
            "MagicDNS",
        ),
    ] {
        let host = Fake::new(vec![reply(value)]);
        assert!(prepare(&host).unwrap_err().contains(expected));
        assert_eq!(host.calls.borrow().len(), 1);
    }
    let host = Fake::new(vec![Err("Tailscale is not installed".into())]);
    assert!(prepare(&host).unwrap_err().contains("not installed"));
}

#[test]
fn logged_out_nonzero_status_is_still_actionable() {
    let host = Fake::new(vec![Ok(Output {
        success: false,
        stdout: "{\"BackendState\":\"NeedsLogin\"}".into(),
        stderr: "logged out".into(),
    })]);
    assert!(prepare(&host).unwrap_err().contains("log in"));
}

#[test]
fn serve_sets_persistent_https_with_the_given_loopback_port() {
    let host = Fake::new(vec![running(), reply(json!({})), reply(json!({}))]);
    serve(&host, "127.0.0.1:7421".parse().unwrap()).unwrap();
    assert_eq!(
        host.calls.borrow()[2],
        vec!["serve", "--bg", "--https=443", "http://127.0.0.1:7421"]
    );
}

#[test]
fn serve_can_reapply_its_proxy_but_refuses_other_handlers_tcp_and_funnel() {
    let host = Fake::new(vec![
        running(),
        reply(proxy("http://127.0.0.1:7420")),
        reply(json!({})),
    ]);
    serve(&host, DEFAULT_BIND).unwrap();
    for config in [
        proxy("http://127.0.0.1:8888"),
        json!({"TCP":{"443":{"TCPForward":"127.0.0.1:8888"}}}),
        json!({"AllowFunnel":{"my-mac-1.tail123.ts.net:443":true}}),
    ] {
        let host = Fake::new(vec![running(), reply(config)]);
        assert!(serve(&host, DEFAULT_BIND).is_err());
        assert_eq!(host.calls.borrow().len(), 2, "no mutation after refusal");
    }
}

#[test]
fn stop_targets_only_its_root_handler_and_never_resets_or_logs_out() {
    let host = Fake::new(vec![
        running(),
        reply(proxy("http://127.0.0.1:7420")),
        reply(json!({})),
    ]);
    stop(&host, DEFAULT_BIND).unwrap();
    assert_eq!(
        host.calls.borrow()[2],
        vec!["serve", "--bg", "--https=443", "--set-path=/", "off"]
    );
    let host = Fake::new(vec![running(), reply(json!({}))]);
    stop(&host, DEFAULT_BIND).unwrap();
    assert_eq!(host.calls.borrow().len(), 2);
    let host = Fake::new(vec![running(), reply(proxy("http://127.0.0.1:9999"))]);
    assert!(stop(&host, DEFAULT_BIND).unwrap_err().contains("unchanged"));
    assert_eq!(host.calls.borrow().len(), 2);
}

#[test]
fn bad_status_config_and_command_failures_do_not_succeed() {
    let host = Fake::new(vec![Ok(Output {
        success: true,
        stdout: "not JSON".into(),
        stderr: String::new(),
    })]);
    assert!(prepare(&host).unwrap_err().contains("status"));
    let host = Fake::new(vec![
        running(),
        Ok(Output {
            success: true,
            stdout: "bad config".into(),
            stderr: String::new(),
        }),
    ]);
    assert!(serve(&host, DEFAULT_BIND).unwrap_err().contains("invalid"));
    let host = Fake::new(vec![
        running(),
        reply(json!({})),
        Ok(Output {
            success: false,
            stdout: String::new(),
            stderr: "permission denied".into(),
        }),
    ]);
    assert!(
        serve(&host, DEFAULT_BIND)
            .unwrap_err()
            .contains("permission denied")
    );
}

fn printed(stdout: &str) -> Result<Output, String> {
    Ok(Output {
        success: true,
        stdout: stdout.into(),
        stderr: String::new(),
    })
}

fn failed(stderr: &str) -> Result<Output, String> {
    Ok(Output {
        success: false,
        stdout: String::new(),
        stderr: stderr.into(),
    })
}

/// Each message keeps today's text and adds why, so a person can act on it.
#[test]
fn unreadable_status_says_why() {
    let error = prepare(&Fake::new(vec![printed("{}")])).unwrap_err();
    assert!(
        error.starts_with(
            "could not read Tailscale status; start Tailscale and log in, then rerun `kkt setup`"
        ),
        "{error}"
    );
    assert!(
        error.ends_with("(its output could not be read: unexpected data at line 1 column 2)"),
        "{error}"
    );
    let error = prepare(&Fake::new(vec![printed("{\"BackendState\":")])).unwrap_err();
    assert!(
        error.ends_with("(its output could not be read: unexpected end at line 1 column 16)"),
        "{error}"
    );
}

/// serde_json's own text quotes values it did not expect; the status may
/// hold private data, so the message must never repeat it.
#[test]
fn unreadable_status_never_repeats_its_values() {
    let marker = "PRIVATE_STATUS_MARKER";
    for stdout in [
        format!("{{\"BackendState\":\"Running\",\"Self\":\"{marker}\"}}"),
        format!("{{\"BackendState\":\"Running\",\"Self\":{{\"DNSName\":[\"{marker}\"]}}}}"),
        format!("{{\"BackendState\":\"Running\",\"CertDomains\":\"{marker}\"}}"),
        format!("{{\"BackendState\":{marker}}}"),
    ] {
        let error = prepare(&Fake::new(vec![printed(&stdout)])).unwrap_err();
        assert!(
            error.starts_with("could not read Tailscale status;"),
            "{error}"
        );
        assert!(error.contains(" at line 1 column "), "{error}");
        assert!(!error.contains(marker), "{error}");
    }
}

#[test]
fn failed_status_says_what_tailscale_said() {
    let host = Fake::new(vec![Ok(Output {
        success: false,
        stdout: running().unwrap().stdout,
        stderr: "permission denied\n".into(),
    })]);
    let error = prepare(&host).unwrap_err();
    assert_eq!(
        error,
        "Tailscale status failed; check `tailscale status` before rerunning `kkt setup` \
         (Tailscale said: permission denied)"
    );
}

#[test]
fn failed_serve_status_says_what_tailscale_said() {
    let host = Fake::new(vec![running(), failed("access denied\n")]);
    let error = serve(&host, DEFAULT_BIND).unwrap_err();
    assert_eq!(
        error,
        "could not inspect Tailscale Serve; run `tailscale serve status` and check permissions \
         (Tailscale said: access denied)"
    );
    // With nothing on stderr, the message is today's alone.
    let host = Fake::new(vec![running(), failed("")]);
    let error = serve(&host, DEFAULT_BIND).unwrap_err();
    assert_eq!(
        error,
        "could not inspect Tailscale Serve; run `tailscale serve status` and check permissions"
    );
}

#[test]
fn invalid_serve_config_says_why() {
    let host = Fake::new(vec![running(), printed("bad config")]);
    let error = serve(&host, DEFAULT_BIND).unwrap_err();
    assert_eq!(
        error,
        "Tailscale Serve returned invalid configuration JSON: syntax error at line 1 column 1"
    );
    let error = served(&Fake::new(vec![printed("bad config")]), DEFAULT_BIND).unwrap_err();
    assert_eq!(
        error,
        "Tailscale Serve returned invalid configuration JSON: syntax error at line 1 column 1"
    );
}

#[test]
fn non_loopback_and_ephemeral_bind_are_refused_before_running_commands() {
    let host = Fake::new(vec![]);
    assert!(serve(&host, "0.0.0.0:7420".parse().unwrap()).is_err());
    assert!(stop(&host, "127.0.0.1:0".parse().unwrap()).is_err());
    assert!(host.calls.borrow().is_empty());
}

#[test]
fn served_finds_the_node_proxying_to_the_server_from_serves_settings_alone() {
    let target = format!("http://{DEFAULT_BIND}");
    let host = Fake::new(vec![reply(proxy(&target))]);
    let node = served(&host, DEFAULT_BIND).unwrap().expect("served");
    assert_eq!(node.dns_name, "my-mac-1.tail123.ts.net");
    // No login needed: only Serve's settings are read, and nothing changes.
    assert_eq!(
        *host.calls.borrow(),
        vec![vec!["serve", "status", "--json"]]
    );
}

#[test]
fn served_is_none_when_tailscale_proxies_elsewhere_serves_nothing_or_is_not_installed() {
    for answer in [
        reply(proxy("http://127.0.0.1:9999")),
        reply(json!({})),
        Ok(Output {
            success: true,
            stdout: String::new(),
            stderr: String::new(),
        }),
        Err("could not run Tailscale: not found".to_string()),
    ] {
        let host = Fake::new(vec![answer]);
        assert_eq!(served(&host, DEFAULT_BIND).unwrap(), None);
    }
    // A server on another machine's address is never shared this way.
    let host = Fake::new(vec![]);
    assert_eq!(
        served(&host, "0.0.0.0:7420".parse().unwrap()).unwrap(),
        None
    );
}

#[test]
fn served_says_when_serves_settings_cannot_be_read() {
    let failed = Ok(Output {
        success: false,
        stdout: String::new(),
        stderr: "failed to connect to local tailscaled".into(),
    });
    let invalid = Ok(Output {
        success: true,
        stdout: "not json".into(),
        stderr: String::new(),
    });
    let error = served(&Fake::new(vec![failed]), DEFAULT_BIND).unwrap_err();
    assert!(error.contains("local tailscaled"), "{error}");
    assert!(served(&Fake::new(vec![invalid]), DEFAULT_BIND).is_err());
}
