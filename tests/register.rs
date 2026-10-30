//! Identity CLI credentials stay in private files, never arguments or output.
mod common;
#[path = "common/scratch.rs"]
mod scratch;

use kashkeshet::admission::{Admission, SharedKey};
use kashkeshet::watch::{Remote, Server};
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};
use std::path::Path;
use std::process::{Output, Stdio};
use tokio::process::Command;

const KKT: &str = env!("CARGO_BIN_EXE_kkt");
const KEY: &str = "0123456789abcdef0123456789abcdef";

async fn start() -> common::Server {
    let mut config = common::test_config();
    config.admission = Admission::SharedKey(SharedKey::new(KEY, "test").expect("key"));
    common::start_with(config).await
}

fn home(name: &str) -> scratch::Dir {
    let home = scratch::Dir::new(name);
    std::fs::write(home.join("key"), KEY).expect("key file");
    home
}

fn command(url: &str, home: &Path, operation: &str, token_file: &str) -> Command {
    let mut cmd = Command::new(KKT);
    cmd.arg(operation)
        .args(["--url", url])
        .arg("--key-file")
        .arg(home.join("key"))
        .arg("--token-file")
        .arg(home.join(token_file))
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("KKT_TOKEN")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd
}

async fn finish(mut command: Command, secrets: &[&str]) -> Output {
    for arg in command.as_std().get_args() {
        for secret in secrets {
            assert!(
                !arg.to_string_lossy().contains(secret),
                "credential in arguments"
            );
        }
    }
    let output = tokio::time::timeout(std::time::Duration::from_secs(20), command.output())
        .await
        .expect("command finished")
        .expect("run command");
    private_output(&output, secrets);
    output
}

fn private_output(output: &Output, secrets: &[&str]) {
    for bytes in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(bytes);
        for secret in secrets.iter().copied().chain([KEY]) {
            assert!(!text.contains(secret), "credential in output");
        }
    }
}

fn secret(home: &Path, name: &str) -> String {
    let path = home.join(name);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).expect("file").permissions().mode() & 0o777,
            0o600
        );
    }
    let value = std::fs::read_to_string(path).expect("credential file");
    assert!(!value.trim().is_empty(), "empty credential");
    value.trim().to_string()
}

async fn agent_count(server: &common::Server, token: &str) -> usize {
    let client = common::connect_new_with_key(server, KEY).await;
    let result = client
        .call_tool(
            CallToolRequestParams::new("list_agents").with_arguments(
                json!({"format": "json", "token": token})
                    .as_object()
                    .expect("object")
                    .clone(),
            ),
        )
        .await
        .expect("list agents");
    assert_ne!(result.is_error, Some(true));
    let text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
        .collect();
    let value: Value = serde_json::from_str(&text).expect("agents JSON");
    value["agents"].as_array().expect("agents").len()
}

#[tokio::test]
async fn register_and_reclaim_save_private_credentials_and_only_print_the_name() {
    let server = start().await;
    let home = home("register-reclaim");
    let url = format!("{}/mcp", server.base);
    let mut register = command(&url, &home, "register", "token");
    register
        .arg("--recovery-file")
        .arg(home.join("recovery"))
        .args(["--description", "CLI worker"]);
    let output = finish(register, &[]).await;
    assert!(output.status.success(), "registration failed");
    let token = secret(&home, "token");
    let recovery = secret(&home, "recovery");
    private_output(&output, &[&token, &recovery]);
    assert!(output.stderr.is_empty(), "unexpected stderr");
    let name = String::from_utf8(output.stdout).expect("name");
    assert_eq!(name.lines().count(), 1);
    let remote = Remote::new(url.clone(), Some(KEY.into()), token.clone());
    assert!(remote.name().await.is_ok_and(|n| format!("{n}\n") == name));

    let client = common::connect_new_with_key(&server, KEY).await;
    let result = client
        .call_tool(
            CallToolRequestParams::new("list_agents").with_arguments(
                json!({"token": token, "format": "json"})
                    .as_object()
                    .expect("object")
                    .clone(),
            ),
        )
        .await
        .expect("whoami");
    let text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
        .collect();
    let value: Value = serde_json::from_str(&text).expect("whoami JSON");
    assert!(
        value["agents"]
            .as_array()
            .expect("agents")
            .iter()
            .any(|agent| {
                agent["name"].as_str() == Some(name.trim())
                    && agent["description"].as_str() == Some("CLI worker")
            })
    );

    // An existing destination must not rotate the current token.
    let mut refused = command(&url, &home, "reclaim", "token");
    refused
        .args(["--name", name.trim()])
        .arg("--recovery-file")
        .arg(home.join("recovery"));
    assert!(!finish(refused, &[&token, &recovery]).await.status.success());
    assert!(remote.name().await.is_ok());

    for destination in ["replacement", "replacement-again"] {
        #[cfg(unix)]
        if destination == "replacement" {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                home.join("recovery"),
                std::fs::Permissions::from_mode(0o644),
            )
            .expect("mode");
        }
        let mut reclaim = command(&url, &home, "reclaim", destination);
        reclaim
            .args(["--name", name.trim()])
            .arg("--recovery-file")
            .arg(home.join("recovery"));
        let output = finish(reclaim, &[&token, &recovery]).await;
        assert!(output.status.success(), "reclaim failed");
        let replacement = secret(&home, destination);
        private_output(&output, &[&replacement]);
        assert!(output.stdout == name.as_bytes());
        #[cfg(unix)]
        if destination == "replacement" {
            assert!(String::from_utf8_lossy(&output.stderr).contains("is now private"));
            assert_eq!(secret(&home, "recovery"), recovery);
        } else {
            assert!(output.stderr.is_empty());
        }
        assert!(
            Remote::new(url.clone(), Some(KEY.into()), replacement)
                .name()
                .await
                .is_ok()
        );
        assert!(remote.name().await.is_err(), "old token still accepted");
    }
}

#[tokio::test]
async fn existing_files_are_untouched_and_do_not_register_an_identity() {
    let server = start().await;
    let home = home("register-existing");
    let url = format!("{}/mcp", server.base);
    std::fs::write(home.join("existing"), "keep this").expect("existing file");
    let observer = kashkeshet::mcp_client::Client::new(url.clone(), Some(KEY.into()))
        .register(None)
        .await
        .unwrap_or_else(|_| panic!("observer registration"));
    let before = agent_count(&server, &observer.token).await;
    let output = finish(command(&url, &home, "register", "existing"), &[]).await;
    assert!(!output.status.success());
    let mut recovery_exists = command(&url, &home, "register", "new-token");
    recovery_exists
        .arg("--recovery-file")
        .arg(home.join("existing"));
    assert!(!finish(recovery_exists, &[]).await.status.success());
    assert!(!home.join("new-token").exists());
    let mut same_path = command(&url, &home, "register", "same");
    same_path.arg("--recovery-file").arg(home.join("same"));
    assert!(!finish(same_path, &[]).await.status.success());
    assert!(!home.join("same").exists());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(home.join("existing"), home.join("link")).expect("symlink");
        assert!(
            !finish(command(&url, &home, "register", "link"), &[])
                .await
                .status
                .success()
        );
    }
    assert_eq!(
        std::fs::read_to_string(home.join("existing")).expect("existing"),
        "keep this"
    );
    assert_eq!(agent_count(&server, &observer.token).await, before);
}

#[tokio::test]
async fn bad_address_and_key_fail_without_leaking_credentials_or_leaving_files() {
    let server = start().await;
    let home = home("register-bad-connection");
    let url = format!("{}/mcp", server.base);
    std::fs::write(home.join("key"), "wrong-access-key").expect("bad key");
    for address in [url.as_str(), "not a URL"] {
        let mut register = command(address, &home, "register", "token");
        register.arg("--recovery-file").arg(home.join("recovery"));
        let output = finish(register, &["wrong-access-key"]).await;
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!home.join("token").exists());
        assert!(!home.join("recovery").exists());
    }
}

#[tokio::test]
async fn registration_without_optional_flags_and_bad_recovery() {
    let server = start().await;
    let home = home("register-minimal");
    let url = format!("{}/mcp", server.base);
    let output = finish(command(&url, &home, "register", "token"), &[]).await;
    assert!(output.status.success());
    let token = secret(&home, "token");
    private_output(&output, &[&token]);
    let name = String::from_utf8(output.stdout).expect("name");
    let recovery = home.join("bad-recovery");
    std::fs::write(&recovery, "invalid-recovery-code").expect("recovery file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut insecure = command(&url, &home, "reclaim", "replacement");
        insecure
            .args(["--name", name.trim()])
            .arg("--recovery-file")
            .arg(&recovery);
        std::fs::set_permissions(&recovery, std::fs::Permissions::from_mode(0o644)).expect("mode");
        assert!(
            !finish(insecure, &[&token, "invalid-recovery-code"])
                .await
                .status
                .success()
        );
        std::fs::set_permissions(&recovery, std::fs::Permissions::from_mode(0o600)).expect("mode");
    }
    let mut invalid = command(&url, &home, "reclaim", "replacement");
    invalid
        .args(["--name", name.trim()])
        .arg("--recovery-file")
        .arg(&recovery);
    assert!(
        !finish(invalid, &[&token, "invalid-recovery-code"])
            .await
            .status
            .success()
    );
    assert!(!home.join("replacement").exists());
    assert!(
        Remote::new(url, Some(KEY.into()), token)
            .name()
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn register_with_a_name_gets_that_name_or_registers_nothing() {
    let server = start().await;
    let home = home("register-name");
    let url = format!("{}/mcp", server.base);
    let mut named = command(&url, &home, "register", "token");
    named.args(["--name", "api-reviewer-2"]);
    let output = finish(named, &[]).await;
    assert!(output.status.success(), "registration failed");
    let token = secret(&home, "token");
    private_output(&output, &[&token]);
    assert_eq!(output.stdout, b"api-reviewer-2\n");
    let remote = Remote::new(url.clone(), Some(KEY.into()), token.clone());
    assert!(remote.name().await.is_ok_and(|n| n == "api-reviewer-2"));
    let before = agent_count(&server, &token).await;

    // Taken: the server refuses it, and the files reserved for it go.
    let mut taken = command(&url, &home, "register", "taken-token");
    taken
        .args(["--name", "api-reviewer-2"])
        .arg("--recovery-file")
        .arg(home.join("taken-recovery"));
    let output = finish(taken, &[&token]).await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("name_taken"));
    assert!(!home.join("taken-token").exists());
    assert!(!home.join("taken-recovery").exists());

    // Badly shaped or reserved: refused with the reason, before any file
    // is made.
    for (bad, reason) in [
        ("Api-Reviewer", "lowercase letters and digits"),
        ("admin-sentinel", "reserved"),
    ] {
        let mut refused = command(&url, &home, "register", "bad-token");
        refused.args(["--name", bad]);
        let output = finish(refused, &[&token]).await;
        assert!(!output.status.success(), "{bad}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(reason),
            "{bad}"
        );
        assert!(!home.join("bad-token").exists());
    }
    assert_eq!(agent_count(&server, &token).await, before);
}

#[tokio::test]
async fn a_failed_description_keeps_saved_credentials_usable() {
    let server = start().await;
    let home = home("register-description-failure");
    let url = format!("{}/mcp", server.base);
    let mut register = command(&url, &home, "register", "token");
    let description = "x".repeat(common::test_config().limits.max_description_chars + 1);
    register
        .arg("--description")
        .arg(description)
        .arg("--recovery-file")
        .arg(home.join("recovery"));
    let output = finish(register, &[]).await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let token = secret(&home, "token");
    let recovery = secret(&home, "recovery");
    private_output(&output, &[&token, &recovery]);
    assert!(
        Remote::new(url, Some(KEY.into()), token)
            .name()
            .await
            .is_ok()
    );
}
