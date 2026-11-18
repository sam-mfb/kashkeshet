//! Client-side channel selection and mention matching.

use super::{Channel, WatchError};

pub(super) fn channels(
    ids: Option<&[String]>,
    names: &[String],
    visible: &[Channel],
) -> Result<Option<Vec<String>>, WatchError> {
    let mut selected = ids.unwrap_or_default().to_vec();
    for name in names {
        let matches: Vec<_> = visible.iter().filter(|c| c.name == *name).collect();
        match matches.as_slice() {
            [] => return Err(WatchError::NoChannelNamed { name: name.clone() }),
            [channel] => selected.push(channel.id.clone()),
            _ => {
                return Err(WatchError::SeveralChannelsNamed {
                    name: name.clone(),
                    ids: matches.iter().map(|c| c.id.clone()).collect(),
                });
            }
        }
    }
    selected.sort();
    selected.dedup();
    Ok((!selected.is_empty()).then_some(selected))
}

pub(super) fn mentions(body: &str, handle: &str) -> bool {
    // Hyphens belong to handles too: `small-me` must not mention `me`.
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    body.match_indices(handle).any(|(start, matched)| {
        !body[..start].chars().next_back().is_some_and(word)
            && !body[start + matched.len()..]
                .chars()
                .next()
                .is_some_and(word)
    })
}
