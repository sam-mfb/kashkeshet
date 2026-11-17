//! kkt-agent-commands' parts, as kkt answers them: the MCP client
//! (kkt-agent-client-mcp, injected as it is), the key files, this program's
//! path and the terminal, with the local server's facts from the setup and
//! the serve settings.

use crate::commands::watch::WatchOutput;
use crate::machine::client_config::{ClaudeCommand, ClientHost};
use crate::machine::connection::ConnectionReads;
use crate::machine::credentials::CredentialFiles;
use crate::machine::operate::NativeOperate;
use crate::machine::settings::ServeArgs;
use kkt_agent_commands::{AdminKey, AgentParts, KeyText, KeyUnread, Output, Program};
use kkt_clock_system::SystemClock;
use kkt_machine::{Env, Platform};
use kkt_terminal::Terminal;
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
        admin_path: kkt_mcp::MCP_ADMIN_PATH,
        servers: Box::new(kkt_agent_client_mcp::Mcp),
        credentials: Box::new(CredentialFiles),
        key_files: Box::new(KeyFiles),
        clients: Box::new(ClientHost),
        client_command: Box::new(ClaudeCommand),
        program: Box::new(ThisProgram),
        report: Box::new(WatchOutput),
        output: Box::new(TerminalOutput),
        clock: Arc::new(SystemClock),
    }
}

/// Where `kkt key show --admin` finds the admin key: the operator's key
/// finder.
struct AdminKeyHere {
    env: Env,
    platform: Platform,
    serve_args: ServeArgs,
}

impl AdminKey for AdminKeyHere {
    fn path(&self, given: Option<PathBuf>) -> Result<PathBuf, String> {
        // The operator's key finder answers, over the operator's parts on
        // this machine: kkt-agent-commands depends on no other assembly.
        let native =
            NativeOperate::new(&self.env, self.platform).with_service_args(self.serve_args);
        let setup_lanes = native.setup_lanes();
        let install = native.install_parts(&setup_lanes);
        let installed =
            kkt_install_commands::record::InstallReads::new(&install, &self.env, self.platform);
        kkt_operator_commands::keys::selected_key_path(
            &native.operator_parts(&installed),
            &self.env,
            given,
            kkt_operator_commands::keys::KeyKind::Admin,
        )
    }
}

/// kkt-keys' key files.
struct KeyFiles;

impl KeyText for KeyFiles {
    fn read(&self, path: &Path) -> Result<String, KeyUnread> {
        kkt_keys::KeyFiles::read_key(&kkt_keys_os::OsKeyFiles, path).map_err(|e| KeyUnread {
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
