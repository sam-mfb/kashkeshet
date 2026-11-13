//! One key shared by every caller, kept only as a hash.

use crate::{Credentials, Decision, Mode, Principal, Refusal, ServerKey};
use kkt_server_keys::{SettingError, check_access_key};

/// One of the server's keys, shared by every caller that holds it. It keeps
/// which key it is, a SHA-256 of the key (not the key), and where the key
/// came from.
pub struct SharedKey {
    key: ServerKey,
    digest: [u8; 32],
    source: String,
}

impl SharedKey {
    /// The server's key `key`, in `text`, without surrounding whitespace
    /// (so a file's final newline is not part of it). `source` says where it
    /// came from, such as the file path. Fails if the key is not one
    /// `check_access_key` accepts.
    pub fn new(
        key: ServerKey,
        text: &str,
        source: impl Into<String>,
    ) -> Result<SharedKey, SettingError> {
        let text = text.trim();
        check_access_key(text)?;
        Ok(SharedKey {
            key,
            digest: kkt_hash::sha256(text.as_bytes()),
            source: source.into(),
        })
    }

    /// Where the key came from.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Whether `presented` is the key. Takes the same time whatever
    /// `presented` is: both sides are hashed to the same length, and every
    /// byte is compared.
    pub fn matches(&self, presented: &[u8]) -> bool {
        let presented: [u8; 32] = kkt_hash::sha256(presented);
        let diff = self
            .digest
            .iter()
            .zip(presented.iter())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b));
        std::hint::black_box(diff) == 0
    }
}

/// The shared-key mode: a caller is let in as the holder of this key, the
/// [`ServerKey`] it is, when it presents the key as a bearer token.
impl Mode for SharedKey {
    fn admit(&self, credentials: &Credentials) -> Decision {
        match credentials.bearer() {
            Ok(token) if self.matches(token) => Decision::Admitted(Principal::KeyHolder(self.key)),
            Ok(_) => Decision::Refused(Refusal::WrongKey),
            Err(refusal) => Decision::Refused(refusal),
        }
    }
}

impl std::fmt::Debug for SharedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedKey")
            .field("key", &self.key)
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn surrounding_whitespace_is_not_part_of_the_key() {
        let key = SharedKey::new(ServerKey::Access, &format!("  {KEY}\n"), "file").expect("key");
        assert!(key.matches(KEY.as_bytes()));
        assert!(!key.matches(format!("{KEY}\n").as_bytes()));
    }

    #[test]
    fn keys_config_refuses_are_refused() {
        for text in ["", "\n", "short", &format!("{KEY} {KEY}")] {
            let err = SharedKey::new(ServerKey::Access, text, "file").expect_err(text);
            assert_eq!(err.name, "access_key_file");
        }
    }

    #[test]
    fn a_generated_key_is_a_shared_key_that_matches_it() {
        use kkt_entropy::testing::SeqEntropy;
        let first = kkt_server_keys::generate(&SeqEntropy::new(3));
        let key =
            SharedKey::new(ServerKey::Access, &first, "file").expect("a generated key is valid");
        assert!(key.matches(first.as_bytes()));
    }

    #[test]
    fn the_mode_admits_the_keys_holder_only() {
        let key = SharedKey::new(ServerKey::Web, KEY, "file").expect("key");
        let admit = |header: Option<String>| {
            key.admit(&Credentials::from_authorization(
                header.as_deref().map(str::as_bytes),
            ))
        };
        assert_eq!(
            admit(Some(format!("Bearer {KEY}"))),
            Decision::Admitted(Principal::KeyHolder(ServerKey::Web))
        );
        assert_eq!(
            admit(Some("Bearer 0123456789abcdef0123456789abcdeX".into())),
            Decision::Refused(Refusal::WrongKey)
        );
        assert_eq!(admit(None), Decision::Refused(Refusal::NoCredentials));
        assert_eq!(
            admit(Some(format!("Basic {KEY}"))),
            Decision::Refused(Refusal::NotBearer)
        );
    }

    #[test]
    fn debug_does_not_show_the_key() {
        let key = SharedKey::new(ServerKey::Access, KEY, "/keys/kkt").expect("key");
        let shown = format!("{key:?}");
        assert!(shown.contains("/keys/kkt"));
        assert!(!shown.contains(KEY));
    }
}
