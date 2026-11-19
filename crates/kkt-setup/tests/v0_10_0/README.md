# Setup records and notes written by kkt 0.10.0

These files are what kkt 0.10.0's own code writes: its `Record::render` and
`Unfinished::render`, at the tag `v0.10.0` (commit 825c6fe). kkt-setup's
record tests read each one with today's code and check that it is the same
record, and that today's `render` writes the same bytes: existing installs
have records like these.

They were made by checking out `v0.10.0` in a scratch worktree, adding the
test below to its `crates/kkt-setup/src/record.rs`, and running

    FIXTURE_OUT=<this folder> cargo test -p kkt-setup v0_10_0_fixtures -- --ignored

Do not edit these files: make them again the same way if a case is added.

```rust

/// Writes what this release's `render` gives, for later releases' tests
/// to pin as records and notes 0.10.0 wrote. Run on demand only.
#[cfg(test)]
mod v0_10_0_fixtures {
    use super::*;

    #[test]
    #[ignore = "writes fixtures; run on demand"]
    fn write_fixtures() {
        let out = std::path::Path::new(env!("FIXTURE_OUT"));
        std::fs::create_dir_all(out).unwrap();
        let record = |mode: Mode, url: &str, key: Option<&str>, clients: Option<Server>| Record {
            mode,
            written_by: "0.10.0".into(),
            url: url.into(),
            key_file: key.map(PathBuf::from),
            clients,
        };
        let records = [
            (
                "record-service.toml",
                record(
                    Mode::Service { tailscale: false },
                    "http://127.0.0.1:7420/mcp",
                    Some("/home/sam/.config/kkt/access-key"),
                    None,
                ),
            ),
            (
                "record-docker-tailscale.toml",
                record(
                    Mode::Docker(DockerSetup {
                        folder: "/home/sam/.local/share/kashkeshet/docker".into(),
                        tailscale: true,
                        hostname: Some("kkt".into()),
                        tailnet: Some("tail1234.ts.net".into()),
                    }),
                    "https://kkt.tail1234.ts.net/mcp",
                    Some("/home/sam/.config/kkt/docker-access-key"),
                    None,
                ),
            ),
            (
                "record-client.toml",
                record(
                    Mode::Client,
                    "https://kkt.tail1234.ts.net/mcp",
                    Some("/home/sam/.config/kkt/client-access-key-ab12cd"),
                    None,
                ),
            ),
            (
                "record-service-switched.toml",
                record(
                    Mode::Service { tailscale: true },
                    "https://kkt.tail1234.ts.net/mcp",
                    Some("/home/sam/.config/kkt/access-key"),
                    Some(Server {
                        url: "https://other.example/mcp".into(),
                        key_file: Some("/home/sam/.config/kkt/client-access-key-ef34gh".into()),
                    }),
                ),
            ),
        ];
        for (name, record) in records {
            std::fs::write(out.join(name), record.render().unwrap()).unwrap();
        }
        let notes = [
            (
                "unfinished-docker-tailscale.toml",
                Unfinished {
                    purpose: Purpose::Docker,
                    tailscale: true,
                    allow_hosts: vec!["kkt.tail1234.ts.net".into()],
                    clients: vec![
                        kkt_client_config::Client::ClaudeCode,
                        kkt_client_config::Client::Codex,
                    ],
                    tailscale_login: Some(TailscaleLogin {
                        hostname: "kkt".into(),
                        auth_key_file: Some("/home/sam/ts-authkey".into()),
                    }),
                    record: None,
                },
            ),
            (
                "unfinished-service.toml",
                Unfinished {
                    purpose: Purpose::Service,
                    tailscale: false,
                    allow_hosts: Vec::new(),
                    clients: vec![kkt_client_config::Client::ClaudeCode],
                    tailscale_login: None,
                    record: Some(record(
                        Mode::Service { tailscale: false },
                        "http://127.0.0.1:7420/mcp",
                        Some("/home/sam/.config/kkt/access-key"),
                        None,
                    )),
                },
            ),
        ];
        for (name, note) in notes {
            std::fs::write(out.join(name), note.render().unwrap()).unwrap();
        }
    }
}
```
