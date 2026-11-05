//! Turns the tool text files (`agent-tools.toml`, `admin-tools.toml`)
//! into Rust constants that `src/tool_text.rs` includes, so a mistake in
//! either file stops the build with a message naming it.
//!
//! The git commit is recorded by `crates/kkt-provenance/build.rs`.

use std::path::Path;

/// Each tool text file and the constant it becomes in `src/tool_text.rs`.
const TOOL_TEXT: [(&str, &str); 2] = [("agent-tools.toml", "AGENT"), ("admin-tools.toml", "ADMIN")];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    write_tool_text()?;
    for path in ["build.rs", "agent-tools.toml", "admin-tools.toml"] {
        println!("cargo:rerun-if-changed={path}");
    }
    Ok(())
}

/// One tool's text, read from a tool text file.
struct ToolText {
    name: String,
    description: String,
    parameters: Vec<(String, String)>,
}

/// Writes `tool_text.rs` in Cargo's output folder: one constant per tool
/// text file, a list of `Tool` values (the type is in `src/tool_text.rs`).
fn write_tool_text() -> Result<(), String> {
    let mut out = String::new();
    for (file, constant) in TOOL_TEXT {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
        let tools = tool_texts(&text).map_err(|e| format!("{file}: {e}"))?;
        out.push_str(&format!("pub(crate) const {constant}: &[Tool] = &[\n"));
        for tool in tools {
            let parameters: String = tool
                .parameters
                .iter()
                .map(|(name, text)| format!("({name:?}, {text:?}), "))
                .collect();
            out.push_str(&format!(
                "    Tool {{ name: {:?}, description: {:?}, parameters: &[{parameters}] }},\n",
                tool.name, tool.description
            ));
        }
        out.push_str("];\n");
    }
    let dir = std::env::var("OUT_DIR").map_err(|e| format!("OUT_DIR: {e}"))?;
    let path = Path::new(&dir).join("tool_text.rs");
    std::fs::write(&path, out).map_err(|e| format!("{}: {e}", path.display()))
}

/// The tools in a tool text file, or what is wrong with it.
fn tool_texts(text: &str) -> Result<Vec<ToolText>, String> {
    let table: toml::Table = text.parse().map_err(|e| format!("{e}"))?;
    table
        .iter()
        .map(|(name, value)| tool_text(name, value))
        .collect()
}

/// One tool's section, `[name]` with `description` and `[name.parameters]`.
fn tool_text(name: &str, value: &toml::Value) -> Result<ToolText, String> {
    let toml::Value::Table(table) = value else {
        return Err(format!("`{name}` must be a section, [{name}]"));
    };
    let mut description = None;
    let mut parameters = Vec::new();
    for (key, value) in table {
        match (key.as_str(), value) {
            ("description", toml::Value::String(text)) => description = Some(fold(text)),
            ("parameters", toml::Value::Table(texts)) => {
                for (parameter, text) in texts {
                    let toml::Value::String(text) = text else {
                        return Err(format!(
                            "[{name}.parameters] `{parameter}` must be text in quotes"
                        ));
                    };
                    parameters.push((parameter.clone(), fold(text)));
                }
            }
            _ => {
                return Err(format!(
                    "[{name}] has `{key}`; a tool has only `description` (text) \
                     and a [{name}.parameters] section"
                ));
            }
        }
    }
    let description = description.ok_or_else(|| format!("[{name}] has no `description`"))?;
    Ok(ToolText {
        name: name.to_string(),
        description,
        parameters,
    })
}

/// Text as agents see it: each line break, with the spaces around it,
/// becomes one space, and the ends are trimmed.
fn fold(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
