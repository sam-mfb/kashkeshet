//! kkt-agent-commands' parts, as kkt answers them: the MCP client, the key
//! files, this program's path and the terminal, with the local server's
//! facts from the setup and the serve settings.

use crate::commands::watch::{WatchOutput, WatchServer};
use crate::machine::client_config::ClientHost;
use crate::machine::connection::ConnectionReads;
use crate::machine::credentials::CredentialFiles;
use crate::machine::keys::{KeyKind, selected_key_path};
use crate::machine::settings::ServeArgs;
use futures::future::BoxFuture;
use kkt_agent_client as client;
use kkt_agent_commands::{
    Address, AdminKey, AgentParts, Avatars, Failure, KeyText, KeyUnread, Output, Program,
    Reclaimed, Registered, Servers,
};
use kkt_clock::Clock;
use kkt_clock_system::SystemClock;
use kkt_machine::{Env, Platform};
use kkt_terminal_os::Terminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The agents' commands' parts on this machine.
pub fn agent_parts(
    env: &Env,
    platform: Platform,
    serve_args: ServeArgs,
) -> AgentParts<ConnectionReads<'_>> {
    AgentParts {
        connection: ConnectionReads::new(env, platform),
        admin_key: Box::new(AdminKeyHere {
            env: env.clone(),
            platform,
            serve_args,
        }),
        servers: Box::new(McpServers),
        credentials: Box::new(CredentialFiles),
        key_files: Box::new(KeyFiles),
        clients: Box::new(ClientHost),
        program: Box::new(ThisProgram),
        report: Box::new(WatchOutput),
        output: Box::new(TerminalOutput),
        clock: Arc::new(SystemClock),
    }
}

/// Where `kkt key show --admin` finds the admin key.
struct AdminKeyHere {
    env: Env,
    platform: Platform,
    serve_args: ServeArgs,
}

impl AdminKey for AdminKeyHere {
    fn path(&self, given: Option<PathBuf>) -> Result<PathBuf, String> {
        selected_key_path(
            &self.env,
            self.platform,
            self.serve_args,
            given,
            KeyKind::Admin,
        )
    }
}

/// kkt-agent-client's MCP client.
struct McpServers;

fn mcp_client(at: Address) -> client::Client {
    let client = client::Client::new(at.url, at.key);
    match at.key_name {
        Some(name) => client.describe_key(name),
        None => client,
    }
}

impl Servers for McpServers {
    fn avatars(&self, at: Address) -> Box<dyn Avatars> {
        Box::new(McpAvatars(mcp_client(at)))
    }

    fn agent(&self, at: Address, token: String) -> Box<dyn kkt_watch::Server> {
        Box::new(WatchServer::Agent(client::Remote::from_client(
            mcp_client(at),
            token,
            client::Settings::DEFAULT,
        )))
    }

    fn admin(&self, at: Address, clock: Arc<dyn Clock>) -> Box<dyn kkt_watch::Server> {
        Box::new(WatchServer::Admin(client::AdminRemote::new(
            mcp_client(at),
            clock,
            client::Settings::DEFAULT,
        )))
    }
}

struct McpAvatars(client::Client);

impl Avatars for McpAvatars {
    fn register<'a>(
        &'a self,
        handle: Option<&'a str>,
        lane: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Registered, Failure>> {
        Box::pin(async move {
            self.0
                .register(handle, lane)
                .await
                .map(|r| Registered {
                    handle: r.handle,
                    token: r.token,
                    recovery_code: r.recovery_code,
                })
                .map_err(failure)
        })
    }

    fn reclaim<'a>(
        &'a self,
        handle: &'a str,
        recovery_code: &'a str,
    ) -> BoxFuture<'a, Result<Reclaimed, Failure>> {
        Box::pin(async move {
            self.0
                .reclaim(handle, recovery_code)
                .await
                .map(|r| Reclaimed {
                    handle: r.handle,
                    token: r.token,
                })
                .map_err(failure)
        })
    }

    fn describe<'a>(&'a self, token: &'a str, text: &'a str) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move { self.0.set_description(token, text).await.map_err(failure) })
    }
}

/// The client's failure, without the details the commands never show.
fn failure(failure: client::Failure) -> Failure {
    match failure {
        client::Failure::Lost(_) => Failure::Lost,
        client::Failure::KeyRefused { .. } => Failure::KeyRefused,
        client::Failure::Refused { code, message } => Failure::Refused { code, message },
    }
}

/// kkt-admission-os' key files.
struct KeyFiles;

impl KeyText for KeyFiles {
    fn read(&self, path: &Path) -> Result<String, KeyUnread> {
        kkt_admission_os::KeyFiles::read(path).map_err(|e| KeyUnread {
            missing: e.error.kind() == std::io::ErrorKind::NotFound,
            reason: e.error.to_string(),
        })
    }
}

struct ThisProgram;

impl Program for ThisProgram {
    fn kkt_path(&self) -> Result<PathBuf, String> {
        crate::process::kkt_path()
    }
}

struct TerminalOutput;

impl Output for TerminalOutput {
    fn output(&self, text: &str) {
        Terminal.output(format_args!("{text}"));
    }
}
