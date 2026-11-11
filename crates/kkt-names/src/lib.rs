//! Agents' handles, channel IDs, the name packs, and the operator's and
//! admins' names. Random handles and IDs are drawn through kkt-entropy's
//! injected [`Entropy`].

use kkt_entropy::Entropy;

/// Characters used in channel IDs.
const ID_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

pub const CHANNEL_PREFIX: &str = "ch_";

/// A pair of word lists built into the binary. Stored handles never
/// depend on which pack is selected for future registrations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NamePack {
    name: &'static str,
    first: &'static [&'static str],
    second: &'static [&'static str],
}

include!(concat!(env!("OUT_DIR"), "/name_packs.rs"));

impl NamePack {
    pub fn name(self) -> &'static str {
        self.name
    }

    pub fn find(name: &str) -> Option<Self> {
        PACKS.iter().find(|pack| pack.name == name).copied()
    }

    /// Draw a handle from this pack, independently of who asks.
    pub fn agent_handle(self, entropy: &dyn Entropy) -> String {
        let first = self.first[entropy.below(self.first.len())];
        let second = self.second[entropy.below(self.second.len())];
        format!("{first}-{second}")
    }
}

/// The name the human operator's messages carry. No agent can have it as
/// its handle: drawn handles always hold a hyphen, and [`check_proposed_handle`] refuses
/// it. The database relies on this too
/// (`tests/migrations/sql/0008_operator_messages.sql`).
pub const OPERATOR: &str = "operator";

/// Whether `name`, as a message's or a change's author, is the human
/// operator rather than an agent. The one place this is decided.
pub fn is_operator(name: &str) -> bool {
    name == OPERATOR
}

/// What every admin agent's name starts with. No agent's handle can: drawn
/// handles come from the name packs, whose words are never `admin`, and
/// [`check_proposed_handle`] refuses it. So an admin agent can't be taken for
/// an agent, or for the operator.
pub const ADMIN_PREFIX: &str = "admin-";

/// The longest handle an agent may propose at `register`. Every handle
/// the server draws, numbered forms included, fits too (tested below).
pub const MAX_HANDLE_CHARS: usize = 40;

/// Checks a handle an agent proposes for its avatar: ASCII letters of
/// either case, digits and underscores, in words joined by single hyphens,
/// at most `max_chars` in all, and neither [`OPERATOR`] nor starting with
/// [`ADMIN_PREFIX`], in any case. Every drawn handle passes. Whether another
/// avatar has it is not checked here. The error says which rule the handle
/// breaks.
pub fn check_proposed_handle(name: &str, max_chars: usize) -> Result<(), String> {
    if !handle_words(name) {
        return Err(format!(
            "the handle must be letters, digits and underscores, in words joined by single hyphens, such as `api-reviewer-2` or `Api_Reviewer-2`; got {name:?}"
        ));
    }
    if name.len() > max_chars {
        return Err(format!("the handle must be at most {max_chars} characters"));
    }
    // In any case, so that no handle can pass for the operator or an admin.
    let lower = name.to_ascii_lowercase();
    if lower == OPERATOR || lower.starts_with(ADMIN_PREFIX) {
        return Err(format!(
            "`{OPERATOR}` and handles starting `{ADMIN_PREFIX}` are reserved, in any case"
        ));
    }
    Ok(())
}

/// Whether `name` is ASCII letters, digits and underscores, in one or more
/// words joined by single hyphens.
fn handle_words(name: &str) -> bool {
    !name.is_empty()
        && name.split('-').all(|word| {
            !word.is_empty() && word.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
}

/// Whether `name` is lowercase ASCII letters and digits, in one or more
/// words joined by single hyphens.
fn plain_words(name: &str) -> bool {
    !name.is_empty()
        && name.split('-').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// The longest lane name. Lane names follow the handle rules.
pub const MAX_LANE_NAME_CHARS: usize = MAX_HANDLE_CHARS;

/// Checks a lane's name: lowercase ASCII letters and digits, in words
/// joined by single hyphens, at most `max_chars`. That is narrower than a
/// handle: no capitals or underscores. Lane names are separate from
/// handles: no name is reserved, and a lane may share its name with an
/// avatar. The error says
/// which rule the name breaks.
pub fn check_lane_name(name: &str, max_chars: usize) -> Result<(), String> {
    if !plain_words(name) {
        return Err(format!(
            "the lane name must be lowercase letters and digits, in words joined by single hyphens, such as `orchestrator` or `code-reviewer`; got {name:?}"
        ));
    }
    if name.len() > max_chars {
        return Err(format!(
            "the lane name must be at most {max_chars} characters"
        ));
    }
    Ok(())
}

/// The longest name of a memory entry. Entry names follow the handle rules.
pub const MAX_MEMORY_NAME_CHARS: usize = MAX_HANDLE_CHARS;

/// Checks the name of a memory entry: lowercase ASCII letters and digits,
/// in words joined by single hyphens, at most `max_chars`. Unlike a
/// handle, no capitals or underscores; no name is reserved. The error says
/// which rule the name breaks.
pub fn check_memory_name(name: &str, max_chars: usize) -> Result<(), String> {
    if !plain_words(name) {
        return Err(format!(
            "the entry name must be lowercase letters and digits, in words joined by single hyphens, such as `handoff` or `open-questions`; got {name:?}"
        ));
    }
    if name.len() > max_chars {
        return Err(format!(
            "the entry name must be at most {max_chars} characters"
        ));
    }
    Ok(())
}

/// The longest admin agent name, its `admin-` prefix included.
pub const MAX_ADMIN_NAME_CHARS: usize = 40;

/// The name of the admin agent holding the admin key, given when the key is
/// made, such as `admin-sentinel`. Changes made with the key are announced
/// as the operator's, via this name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminName(String);

impl AdminName {
    /// `given`, with [`ADMIN_PREFIX`] in front unless it starts with it. The
    /// rest is lowercase letters and digits, in words joined by single
    /// hyphens; the whole name has at most `max_chars`.
    pub fn new(given: &str, max_chars: usize) -> Result<AdminName, String> {
        let given = given.trim();
        let rest = given.strip_prefix(ADMIN_PREFIX).unwrap_or(given);
        let plain = !rest.is_empty()
            && rest.split('-').all(|word| {
                !word.is_empty()
                    && word
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            });
        if !plain {
            return Err(format!(
                "the admin name must be lowercase letters and digits, in words joined by single hyphens, such as `sentinel` (which becomes `{ADMIN_PREFIX}sentinel`); got {given:?}"
            ));
        }
        let name = format!("{ADMIN_PREFIX}{rest}");
        if name.chars().count() > max_chars {
            return Err(format!(
                "the admin name must be at most {max_chars} characters, `{ADMIN_PREFIX}` included"
            ));
        }
        Ok(AdminName(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The text of a name file that ties this name to the admin key `key`
    /// alone: the name, then a line with a hash of the key (never the key).
    /// Surrounding whitespace is not part of the key, as for `SharedKey`.
    pub fn bound_to(&self, key: &str) -> String {
        format!("{}\n{KEY_HASH_LABEL}{}\n", self.0, key_hash(key))
    }

    /// The name in the name file `text` if it is tied to the admin key
    /// `key`; `None` if it is tied to another key, as when the key file was
    /// replaced without its name. An error says what is wrong with `text`.
    /// The name is checked as [`AdminName::new`] checks it, with at most
    /// `max_chars`.
    pub fn bound_in(text: &str, key: &str, max_chars: usize) -> Result<Option<AdminName>, String> {
        let mut lines = text.lines();
        let name = AdminName::new(lines.next().unwrap_or(""), max_chars)?;
        let Some(hash) = lines.next().and_then(|l| l.strip_prefix(KEY_HASH_LABEL)) else {
            return Err(format!(
                "its second line must be `{KEY_HASH_LABEL}` and a hash of the admin key"
            ));
        };
        if lines.any(|l| !l.trim().is_empty()) {
            return Err("it must have only the name and the key's hash".into());
        }
        Ok((hash.trim() == key_hash(key)).then_some(name))
    }
}

/// How a name file's line with the key's hash starts.
const KEY_HASH_LABEL: &str = "key-sha256: ";

/// A SHA-256 of `key`, without surrounding whitespace, in hex. A fixed
/// prefix keeps it from equalling a hash of the key made for anything else.
fn key_hash(key: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(format!("kkt-admin-name:{}", key.trim()).as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl std::fmt::Display for AdminName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A random adjective-animal pair from [`ANIMALS`], such as `amber-falcon`.
pub fn agent_handle(entropy: &dyn Entropy) -> String {
    ANIMALS.agent_handle(entropy)
}

/// The handle to try after `base` clashes `attempt` times. Attempt 1 gives
/// `base-2`, attempt 2 gives `base-3`, and so on.
pub fn numbered_handle(base: &str, attempt: u32) -> String {
    format!("{base}-{}", attempt + 1)
}

/// Random characters in a channel ID, after its `ch_` prefix.
pub const CHANNEL_RANDOM_LEN: usize = 8;
/// How many random channel IDs to try before giving up.
pub const MAX_CHANNEL_ID_ATTEMPTS: u32 = 100;

/// A random channel ID of [`CHANNEL_PREFIX`] and `random_len` characters,
/// such as `ch_7k2m9q4x`.
pub fn channel_id(entropy: &dyn Entropy, random_len: usize) -> String {
    format!(
        "{CHANNEL_PREFIX}{}",
        entropy.random_chars(ID_CHARS, random_len)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use kkt_entropy::testing::SeqEntropy;
    const DEFAULT_PACK: NamePack = ANIMALS;

    #[test]
    fn only_the_operators_name_is_the_operator() {
        assert!(is_operator(OPERATOR));
        assert!(!is_operator("amber-falcon"));
        assert!(!is_operator("Operator"));
        assert!(!is_operator("admin-sentinel"));
    }
    /// The longest name the tests allow. The limits above are at least
    /// this, so what passes here passes there (checked below).
    const MAX_CHARS: usize = 40;

    #[test]
    fn every_pack_has_plain_unique_words_and_distinct_name_draws() {
        for pack in PACKS {
            for words in [pack.first, pack.second] {
                assert!(words.len() > ID_CHARS.len(), "{}", pack.name);
                let mut seen = std::collections::BTreeSet::new();
                for word in words {
                    assert!(!word.is_empty());
                    assert!(word.bytes().all(|b| b.is_ascii_lowercase()), "{word}");
                    assert!(seen.insert(word), "duplicate {word}");
                    assert!(
                        ![
                            "operator",
                            "admin",
                            "root",
                            "system",
                            "sudo",
                            "daemon",
                            "kernel",
                            "server",
                            "owner",
                            "user",
                            "register",
                            "unregister",
                            "send",
                            "read",
                            "join",
                            "leave"
                        ]
                        .contains(word)
                    );
                }
            }
        }
        assert!(DEFAULT_PACK.first.len() >= 200);
        assert!(DEFAULT_PACK.second.len() >= 200);
    }

    #[test]
    fn default_pack_keeps_every_original_word() {
        let first = "amber ashen bold brave brisk calm clever copper coral crisp dapper eager early fair fierce gentle gilded glad golden grand hazel hidden honest humble icy ivory jade jolly keen kind lively lucky lunar mellow misty noble olive patient plucky polar proud quick quiet rapid rosy rustic sandy scarlet silent silver slate sly snowy solar steady stormy sunny swift tawny tidy velvet vivid wise witty";
        let second = "badger bear beaver bison bobcat crane crow deer dingo dolphin eagle egret elk falcon ferret finch fox gecko gibbon goose hare hawk heron ibis jackal jaguar koala lark lemur lion lynx magpie marten mink moose newt ocelot orca osprey otter owl panda panther parrot pelican puffin quail raven robin salmon seal shrike sparrow stoat swan tapir tern tiger toucan trout turtle viper walrus wren";
        for (old, words) in [(first, DEFAULT_PACK.first), (second, DEFAULT_PACK.second)] {
            for word in old.split_whitespace() {
                assert!(words.contains(&word), "lost original word {word}");
            }
        }
    }

    #[test]
    fn no_agent_handle_is_the_operators() {
        // Every handle `agent_handle` can give, and its numbered forms.
        for pack in PACKS {
            for adjective in pack.first {
                for animal in pack.second {
                    let name = format!("{adjective}-{animal}");
                    for candidate in [name.clone(), numbered_handle(&name, 1)] {
                        assert!(candidate.contains('-'), "{candidate}");
                        assert_ne!(candidate, OPERATOR);
                        assert!(!candidate.starts_with(ADMIN_PREFIX), "{candidate}");
                    }
                }
            }
        }
        assert!(!OPERATOR.contains('-'));
    }

    #[test]
    fn drawn_handles_stay_within_the_handle_limit() {
        // Every drawn handle, numbered up to attempt 10,000, passes the
        // handle check at `MAX_CHARS` (tested below). The limit must stay
        // at least that, or a drawn handle could be refused. kkt-rules
        // checks that it tries no more than 10,000 numbers.
        const { assert!(MAX_HANDLE_CHARS >= MAX_CHARS) };
    }

    #[test]
    fn the_admin_name_text_keeps_its_number() {
        let longest = "a".repeat(MAX_ADMIN_NAME_CHARS - ADMIN_PREFIX.len());
        assert!(AdminName::new(&longest, MAX_ADMIN_NAME_CHARS).is_ok());
        assert_eq!(
            AdminName::new(&format!("{longest}a"), MAX_ADMIN_NAME_CHARS),
            Err("the admin name must be at most 40 characters, `admin-` included".to_string())
        );
    }

    #[test]
    fn every_drawn_handle_would_pass_as_a_proposed_one() {
        let last = 10_000;
        for pack in PACKS {
            for adjective in pack.first {
                for animal in pack.second {
                    let name = format!("{adjective}-{animal}");
                    for candidate in [name.clone(), numbered_handle(&name, last)] {
                        assert_eq!(
                            check_proposed_handle(&candidate, MAX_CHARS),
                            Ok(()),
                            "{candidate}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn proposed_handles_are_plain_short_and_not_reserved() {
        let longest = "a".repeat(MAX_CHARS);
        for good in [
            "api-reviewer-2",
            "worker",
            "a",
            "7",
            "x-1-y",
            "Api-reviewer",
            "api_reviewer",
            "COOL_PROJECT_Google_oauth_REVIEW",
            "_",
            "a_-_b",
            &longest,
        ] {
            assert_eq!(check_proposed_handle(good, MAX_CHARS), Ok(()), "{good:?}");
        }
        let shape = "letters, digits and underscores";
        let too_long = format!("{longest}a");
        for (bad, rule) in [
            ("", shape),
            ("api reviewer", shape),
            ("api.reviewer", shape),
            ("-api", shape),
            ("api-", shape),
            ("api--reviewer", shape),
            (" api", shape),
            ("ünï", shape),
            (too_long.as_str(), "at most 40 characters"),
            (OPERATOR, "reserved"),
            ("Operator", "reserved"),
            ("OPERATOR", "reserved"),
            ("admin-sentinel", "reserved"),
            ("Admin-sentinel", "reserved"),
            ("ADMIN-X", "reserved"),
            ("aDmIn-x_y", "reserved"),
            ("admin-", shape),
        ] {
            let err = check_proposed_handle(bad, MAX_CHARS).expect_err(bad);
            assert!(err.contains(rule), "{bad:?}: {err}");
        }
        // Only the prefix is reserved, not every handle holding the word.
        assert_eq!(check_proposed_handle("admin", MAX_CHARS), Ok(()));
        assert_eq!(check_proposed_handle("my-admin-2", MAX_CHARS), Ok(()));
        assert_eq!(check_proposed_handle("operator-2", MAX_CHARS), Ok(()));
        assert_eq!(check_proposed_handle("Operator_2", MAX_CHARS), Ok(()));
        assert_eq!(check_proposed_handle("admin_x", MAX_CHARS), Ok(()));
    }

    #[test]
    fn lane_and_entry_names_stay_lowercase_with_no_underscores() {
        for bad in ["Handoff", "open_questions"] {
            assert!(check_lane_name(bad, MAX_CHARS).is_err(), "{bad}");
            assert!(check_memory_name(bad, MAX_CHARS).is_err(), "{bad}");
        }
    }

    #[test]
    fn each_check_takes_its_maximum_from_the_caller() {
        // At the maximum given, and one over it, with the number in the text.
        type Check = fn(&str, usize) -> Result<(), String>;
        let checks: [(Check, &str); 3] = [
            (
                check_proposed_handle,
                "the handle must be at most 5 characters",
            ),
            (
                check_lane_name,
                "the lane name must be at most 5 characters",
            ),
            (
                check_memory_name,
                "the entry name must be at most 5 characters",
            ),
        ];
        for (check, text) in checks {
            assert_eq!(check("abcde", 5), Ok(()));
            assert_eq!(check("abcdef", 5), Err(text.to_string()));
        }
        assert!(AdminName::new("abc", 9).is_ok());
        assert_eq!(
            AdminName::new("abcd", 9),
            Err("the admin name must be at most 9 characters, `admin-` included".to_string())
        );
        let text = AdminName::new("abc", 9).unwrap().bound_to("key");
        assert!(AdminName::bound_in(&text, "key", 9).is_ok());
        assert!(AdminName::bound_in(&text, "key", 8).is_err());
    }

    #[test]
    fn admin_names_start_with_the_prefix_and_are_plain_words() {
        let name = |given: &str| AdminName::new(given, MAX_CHARS).map(|n| n.to_string());
        assert_eq!(name("sentinel").as_deref(), Ok("admin-sentinel"));
        assert_eq!(name(" admin-sentinel ").as_deref(), Ok("admin-sentinel"));
        assert_eq!(name("night-owl-2").as_deref(), Ok("admin-night-owl-2"));
        let longest = "a".repeat(MAX_CHARS - ADMIN_PREFIX.len());
        assert!(name(&longest).is_ok());
        for bad in [
            "",
            "admin-",
            "Sentinel",
            "sentinel owl",
            "-owl",
            "owl-",
            "night--owl",
            "operator (via x)",
            "ünï",
            &format!("{longest}a"),
        ] {
            assert!(name(bad).is_err(), "{bad:?}");
        }
        // Never the operator's name: it always has the prefix.
        assert_ne!(name("operator").as_deref(), Ok(OPERATOR));
    }

    #[test]
    fn an_admin_name_belongs_only_to_the_key_it_was_written_for() {
        let first = "0123456789abcdef0123456789abcdef";
        let second = "fedcba9876543210fedcba9876543210";
        let sentinel = AdminName::new("sentinel", MAX_CHARS).unwrap();
        let text = sentinel.bound_to(first);
        assert!(text.starts_with("admin-sentinel\nkey-sha256: "), "{text}");
        assert!(!text.contains(first));

        assert_eq!(
            AdminName::bound_in(&text, first, MAX_CHARS),
            Ok(Some(sentinel.clone()))
        );
        // A key file's final newline is not part of the key.
        assert_eq!(
            AdminName::bound_in(&text, &format!("{first}\n"), MAX_CHARS),
            Ok(Some(sentinel))
        );
        // Another key, or the same file after the key was replaced, has none.
        assert_eq!(AdminName::bound_in(&text, second, MAX_CHARS), Ok(None));

        // A name alone, as an older or hand-written file might hold, is an
        // error, not a name for whatever key is beside it.
        for bad in [
            "admin-sentinel\n",
            "",
            "Night Owl\nkey-sha256: 00\n",
            "admin-sentinel\nsha: 00\n",
            &format!("{text}extra\n"),
        ] {
            assert!(
                AdminName::bound_in(bad, first, MAX_CHARS).is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn shapes() {
        let e = SeqEntropy::new(1);
        let name = agent_handle(&e);
        assert_eq!(name.split('-').count(), 2);
        let id = channel_id(&e, 8);
        assert!(id.starts_with("ch_") && id.len() == 11);
        assert_eq!(channel_id(&e, 3).len(), 6);
        assert_eq!(numbered_handle("amber-falcon", 1), "amber-falcon-2");
    }
}
