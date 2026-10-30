//! Records the git commit the server is built from, for provenance. The
//! server reads it as the `KASHKESHET_GIT_COMMIT` environment variable at
//! compile time. Without git, it is `unknown`.

use std::path::Path;
use std::process::Command;

#[path = "src/provenance/commit.rs"]
mod commit;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    build_name_packs()?;
    let head = git(&["rev-parse", "HEAD"]);
    let dirty = git(&["status", "--porcelain"]).map(|out| !out.trim().is_empty());
    let text = commit::commit_text(head.as_deref(), dirty);
    println!("cargo:rustc-env=KASHKESHET_GIT_COMMIT={text}");

    // Run again when the commit or the working tree changes.
    if let Some(git_dir) = git(&["rev-parse", "--git-dir"]) {
        let git_dir = Path::new(git_dir.trim());
        for name in ["HEAD", "index", "packed-refs"] {
            rerun_if_exists(&git_dir.join(name));
        }
        if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"]) {
            rerun_if_exists(&git_dir.join(branch.trim()));
        }
    }
    for path in [
        "build.rs",
        "Cargo.toml",
        "src",
        "migrations",
        "agent-instructions.md",
        "name-packs",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    Ok(())
}

/// Discover packs at build time so adding one file needs no Rust changes.
fn build_name_packs() -> Result<(), Box<dyn std::error::Error>> {
    let mut paths = std::fs::read_dir("name-packs")?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    let mut generated = String::from("pub(crate) const PACKS: &[NamePack] = &[\n");
    let mut default = None;
    for path in paths {
        if path.extension().and_then(|s| s.to_str()) != Some("txt") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or("invalid pack filename")?;
        if !name.bytes().all(|b| b.is_ascii_lowercase()) || name.is_empty() {
            return Err(format!("invalid pack name: {name}").into());
        }
        let text = std::fs::read_to_string(&path)?;
        let (first, second) = text
            .split_once("\n[second]\n")
            .ok_or("pack needs [second] section")?;
        let first = first
            .strip_prefix("[first]\n")
            .ok_or("pack needs [first] section")?;
        let words = |section: &str| -> Result<String, Box<dyn std::error::Error>> {
            let words: Vec<_> = section.split_whitespace().collect();
            let unique: std::collections::BTreeSet<_> = words.iter().collect();
            if words.len() <= 36
                || words.len() != unique.len()
                || words
                    .iter()
                    .any(|w| !w.bytes().all(|b| b.is_ascii_lowercase()))
            {
                return Err(format!("invalid word list in {}", path.display()).into());
            }
            Ok(format!("&{words:?}"))
        };
        let pack = format!(
            "NamePack {{ name: {name:?}, first: {}, second: {} }}",
            words(first)?,
            words(second)?
        );
        if name == "animals" {
            default = Some(pack.clone());
        }
        generated.push_str(&format!("{pack},\n"));
    }
    generated.push_str("];\n");
    generated.push_str(&format!(
        "pub(crate) const ANIMALS: NamePack = {};\n",
        default.ok_or("animals pack is missing")?
    ));
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").ok_or("OUT_DIR is missing")?);
    std::fs::write(out.join("name_packs.rs"), generated)?;
    Ok(())
}

/// What git prints for `args`, or `None` if git is missing or fails.
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

fn rerun_if_exists(path: &Path) {
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
