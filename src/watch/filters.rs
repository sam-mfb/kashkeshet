//! Client-side channel selection and mention matching.

use super::Channel;

pub(super) fn channels(
    ids: Option<&[String]>,
    names: &[String],
    visible: &[Channel],
) -> Result<Option<Vec<String>>, String> {
    let mut selected = ids.unwrap_or_default().to_vec();
    for name in names {
        let matches: Vec<_> = visible.iter().filter(|c| c.name == *name).collect();
        match matches.as_slice() {
            [] => return Err(format!("no visible channel named {name:?}")),
            [channel] => selected.push(channel.id.clone()),
            _ => {
                let ids: Vec<_> = matches.iter().map(|c| c.id.as_str()).collect();
                return Err(format!(
                    "several visible channels named {name:?}: {}; use --channel with an ID",
                    ids.join(", ")
                ));
            }
        }
    }
    selected.sort();
    selected.dedup();
    Ok((!selected.is_empty()).then_some(selected))
}

pub(super) fn mentions(body: &str, name: &str) -> bool {
    // Hyphens belong to agent names too: `small-me` must not mention `me`.
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    body.match_indices(name).any(|(start, matched)| {
        !body[..start].chars().next_back().is_some_and(word)
            && !body[start + matched.len()..]
                .chars()
                .next()
                .is_some_and(word)
    })
}
