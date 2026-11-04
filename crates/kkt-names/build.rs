//! Turns the name packs in `name-packs/` into Rust constants that
//! `src/lib.rs` includes: `PACKS`, every pack, and `ANIMALS`, the default.
//! A mistake in a pack file stops the build with a message naming it.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    build_name_packs()?;
    for path in ["build.rs", "name-packs"] {
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
    let mut generated = String::from("pub const PACKS: &[NamePack] = &[\n");
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
        "pub const ANIMALS: NamePack = {};\n",
        default.ok_or("animals pack is missing")?
    ));
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").ok_or("OUT_DIR is missing")?);
    std::fs::write(out.join("name_packs.rs"), generated)?;
    Ok(())
}
