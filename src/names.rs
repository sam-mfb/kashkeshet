//! Agent names and channel IDs, and the random characters that IDs and
//! tokens are made of.
//!
//! Randomness is injected through [`Entropy`] so the rest of the system can be
//! tested with a predictable source.

/// A source of random numbers.
pub trait Entropy: Send + Sync {
    /// Returns a uniformly chosen number in `0..n`. `n` is never zero.
    fn below(&self, n: usize) -> usize;
}

/// Entropy from the operating system's cryptographic generator.
pub struct OsEntropy;

impl Entropy for OsEntropy {
    fn below(&self, n: usize) -> usize {
        use rand::RngExt;
        rand::rng().random_range(0..n)
    }
}

/// Entropy that repeats from a seed: the same seed gives the same numbers in
/// the same order. For simulations such as the web preview; never for tokens
/// or keys. (SplitMix64.)
pub struct SeededEntropy(std::sync::atomic::AtomicU64);

impl SeededEntropy {
    pub fn new(seed: u64) -> SeededEntropy {
        SeededEntropy(std::sync::atomic::AtomicU64::new(seed))
    }
}

impl Entropy for SeededEntropy {
    fn below(&self, n: usize) -> usize {
        const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
        let state = self
            .0
            .fetch_add(GAMMA, std::sync::atomic::Ordering::Relaxed)
            .wrapping_add(GAMMA);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z % n as u64) as usize
    }
}

/// Characters used in channel IDs and tokens.
pub(crate) const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

pub const CHANNEL_PREFIX: &str = "ch_";
pub const CHANNEL_RANDOM_LEN: usize = 8;

/// A pair of word lists built into the binary. Stored agent names never
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

    pub(crate) fn find(name: &str) -> Option<Self> {
        PACKS.iter().find(|pack| pack.name == name).copied()
    }

    /// Draw a name from this pack, independently of the caller's identity.
    pub(crate) fn agent_name(self, entropy: &dyn Entropy) -> String {
        let first = self.first[entropy.below(self.first.len())];
        let second = self.second[entropy.below(self.second.len())];
        format!("{first}-{second}")
    }
}

/// The name the human operator's messages carry. No agent can have it:
/// drawn names always hold a hyphen, and [`check_proposed_name`] refuses
/// it. The database relies on this too
/// (`migrations/0008_operator_messages.sql`).
pub const OPERATOR: &str = "operator";

/// What every admin agent's name starts with. No agent's name can: drawn
/// names come from the name packs, whose words are never `admin`, and
/// [`check_proposed_name`] refuses it. So an admin agent can't be taken for
/// an agent, or for the operator.
pub const ADMIN_PREFIX: &str = "admin-";

/// Checks a name an agent proposes for itself, as drawn names look:
/// lowercase ASCII letters and digits, in words joined by single hyphens,
/// at most `MAX_AGENT_NAME_CHARS` in all, and neither [`OPERATOR`] nor
/// starting with [`ADMIN_PREFIX`]. Whether another agent has it is not
/// checked here. The error says which rule the name breaks.
pub fn check_proposed_name(name: &str) -> Result<(), String> {
    use crate::config::MAX_AGENT_NAME_CHARS;
    let plain = !name.is_empty()
        && name.split('-').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        });
    if !plain {
        return Err(format!(
            "the name must be lowercase letters and digits, in words joined by single hyphens, such as `api-reviewer-2`; got {name:?}"
        ));
    }
    if name.len() > MAX_AGENT_NAME_CHARS {
        return Err(format!(
            "the name must be at most {MAX_AGENT_NAME_CHARS} characters"
        ));
    }
    if name == OPERATOR || name.starts_with(ADMIN_PREFIX) {
        return Err(format!(
            "`{OPERATOR}` and names starting `{ADMIN_PREFIX}` are reserved"
        ));
    }
    Ok(())
}

/// The name of the admin agent holding the admin key, given when the key is
/// made, such as `admin-sentinel`. Changes made with the key are announced
/// as the operator's, via this name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminName(String);

impl AdminName {
    /// `given`, with [`ADMIN_PREFIX`] in front unless it starts with it. The
    /// rest is lowercase letters and digits, in words joined by single
    /// hyphens; the whole name has at most `MAX_ADMIN_NAME_CHARS`.
    pub fn new(given: &str) -> Result<AdminName, String> {
        use crate::config::MAX_ADMIN_NAME_CHARS;
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
        if name.chars().count() > MAX_ADMIN_NAME_CHARS {
            return Err(format!(
                "the admin name must be at most {MAX_ADMIN_NAME_CHARS} characters, `{ADMIN_PREFIX}` included"
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
    pub fn bound_in(text: &str, key: &str) -> Result<Option<AdminName>, String> {
        let mut lines = text.lines();
        let name = AdminName::new(lines.next().unwrap_or(""))?;
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

/// A random adjective-animal pair, such as `amber-falcon`.
pub fn agent_name(entropy: &dyn Entropy) -> String {
    crate::config::Limits::DEFAULT.name_pack.agent_name(entropy)
}

/// The name to try after `base` clashes `attempt` times. Attempt 1 gives
/// `base-2`, attempt 2 gives `base-3`, and so on.
pub fn numbered_name(base: &str, attempt: u32) -> String {
    format!("{base}-{}", attempt + 1)
}

/// A random channel ID, such as `ch_7k2m9q4x`.
pub fn channel_id(entropy: &dyn Entropy) -> String {
    format!(
        "{CHANNEL_PREFIX}{}",
        random_chars(entropy, CHANNEL_RANDOM_LEN)
    )
}

pub(crate) fn random_chars(entropy: &dyn Entropy, len: usize) -> String {
    (0..len)
        .map(|_| ALPHABET[entropy.below(ALPHABET.len())] as char)
        .collect()
}

#[cfg(test)]
pub mod testing {
    use super::Entropy;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A predictable entropy source for tests (a simple linear congruential generator).
    pub struct SeqEntropy(AtomicU64);

    impl SeqEntropy {
        pub fn new(seed: u64) -> Self {
            Self(AtomicU64::new(seed))
        }
    }

    impl Entropy for SeqEntropy {
        fn below(&self, n: usize) -> usize {
            let next = self
                .0
                .load(Ordering::Relaxed)
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0.store(next, Ordering::Relaxed);
            ((next >> 33) as usize) % n
        }
    }

    /// Picks the first word for every agent name, so every name clashes, but
    /// draws IDs and tokens normally. Name words come from lists longer than
    /// the ID alphabet, which is how the two kinds of draw are told apart.
    pub struct SameNameEntropy(pub SeqEntropy);

    impl Entropy for SameNameEntropy {
        fn below(&self, n: usize) -> usize {
            if n > super::ALPHABET.len() {
                0
            } else {
                self.0.below(n)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::SeqEntropy;
    const DEFAULT_PACK: NamePack = crate::config::Limits::DEFAULT.name_pack;

    #[test]
    fn every_pack_has_plain_unique_words_and_distinct_name_draws() {
        for pack in PACKS {
            for words in [pack.first, pack.second] {
                assert!(words.len() > ALPHABET.len(), "{}", pack.name);
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
    fn no_agent_name_is_the_operators() {
        // Every name `agent_name` can give, and its numbered forms.
        for pack in PACKS {
            for adjective in pack.first {
                for animal in pack.second {
                    let name = format!("{adjective}-{animal}");
                    for candidate in [name.clone(), numbered_name(&name, 1)] {
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
    fn every_drawn_name_would_pass_as_a_proposed_one() {
        let last = crate::config::MAX_NAME_ATTEMPTS;
        for pack in PACKS {
            for adjective in pack.first {
                for animal in pack.second {
                    let name = format!("{adjective}-{animal}");
                    for candidate in [name.clone(), numbered_name(&name, last)] {
                        assert_eq!(check_proposed_name(&candidate), Ok(()), "{candidate}");
                    }
                }
            }
        }
    }

    #[test]
    fn proposed_names_are_plain_short_and_not_reserved() {
        use crate::config::MAX_AGENT_NAME_CHARS;
        let longest = "a".repeat(MAX_AGENT_NAME_CHARS);
        for good in ["api-reviewer-2", "worker", "a", "7", "x-1-y", &longest] {
            assert_eq!(check_proposed_name(good), Ok(()), "{good:?}");
        }
        let shape = "lowercase letters and digits";
        let too_long = format!("{longest}a");
        for (bad, rule) in [
            ("", shape),
            ("Api-reviewer", shape),
            ("api reviewer", shape),
            ("api_reviewer", shape),
            ("-api", shape),
            ("api-", shape),
            ("api--reviewer", shape),
            (" api", shape),
            ("ünï", shape),
            (too_long.as_str(), "at most 40 characters"),
            (OPERATOR, "reserved"),
            ("admin-sentinel", "reserved"),
            ("admin-", shape),
        ] {
            let err = check_proposed_name(bad).expect_err(bad);
            assert!(err.contains(rule), "{bad:?}: {err}");
        }
        // Only the prefix is reserved, not every name holding the word.
        assert_eq!(check_proposed_name("admin"), Ok(()));
        assert_eq!(check_proposed_name("my-admin-2"), Ok(()));
        assert_eq!(check_proposed_name("operator-2"), Ok(()));
    }

    #[test]
    fn admin_names_start_with_the_prefix_and_are_plain_words() {
        let name = |given: &str| AdminName::new(given).map(|n| n.to_string());
        assert_eq!(name("sentinel").as_deref(), Ok("admin-sentinel"));
        assert_eq!(name(" admin-sentinel ").as_deref(), Ok("admin-sentinel"));
        assert_eq!(name("night-owl-2").as_deref(), Ok("admin-night-owl-2"));
        let longest = "a".repeat(crate::config::MAX_ADMIN_NAME_CHARS - ADMIN_PREFIX.len());
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
        let sentinel = AdminName::new("sentinel").unwrap();
        let text = sentinel.bound_to(first);
        assert!(text.starts_with("admin-sentinel\nkey-sha256: "), "{text}");
        assert!(!text.contains(first));

        assert_eq!(
            AdminName::bound_in(&text, first),
            Ok(Some(sentinel.clone()))
        );
        // A key file's final newline is not part of the key.
        assert_eq!(
            AdminName::bound_in(&text, &format!("{first}\n")),
            Ok(Some(sentinel))
        );
        // Another key, or the same file after the key was replaced, has none.
        assert_eq!(AdminName::bound_in(&text, second), Ok(None));

        // A name alone, as an older or hand-written file might hold, is an
        // error, not a name for whatever key is beside it.
        for bad in [
            "admin-sentinel\n",
            "",
            "Night Owl\nkey-sha256: 00\n",
            "admin-sentinel\nsha: 00\n",
            &format!("{text}extra\n"),
        ] {
            assert!(AdminName::bound_in(bad, first).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn seeded_entropy_repeats_from_its_seed() {
        let draws = |seed| {
            let e = SeededEntropy::new(seed);
            (0..20).map(|_| e.below(1000)).collect::<Vec<_>>()
        };
        assert_eq!(draws(7), draws(7));
        assert_ne!(draws(7), draws(8));
        assert!(draws(7).iter().all(|&n| n < 1000));
    }

    #[test]
    fn shapes() {
        let e = SeqEntropy::new(1);
        let name = agent_name(&e);
        assert_eq!(name.split('-').count(), 2);
        let id = channel_id(&e);
        assert!(id.starts_with("ch_") && id.len() == 11);
        assert_eq!(numbered_name("amber-falcon", 1), "amber-falcon-2");
    }
}
