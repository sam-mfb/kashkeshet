//! Adapting the native terminal and browser link to setup's contracts.

pub mod browser_login;

use kkt_terminal_os::Terminal;

/// The terminal supplied to setup's question rules.
pub struct TerminalLines;

impl kkt_setup::Lines for TerminalLines {
    fn interactive(&self) -> bool {
        Terminal.interactive()
    }
    fn say(&self, text: &str) {
        Terminal.output(format_args!("{text}\n"));
    }
    fn read_line(&self, prompt: &str) -> Result<Option<String>, String> {
        Terminal.read_line(prompt)
    }
}
