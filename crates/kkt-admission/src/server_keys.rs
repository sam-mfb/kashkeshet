//! The server's keys: that its access, web and admin keys are well formed
//! and differ from one another, and the admin name tied to its admin key.
//! It reads no files: the program supplies their texts through
//! [`KeyTexts`], and names the setting in each refusal.

use crate::keys::SettingError;
use crate::{Admission, ServerKey, SharedKey};
use kkt_names::{AdminName, MAX_ADMIN_NAME_CHARS};

/// A key file's text, and where it came from, such as the file's path.
pub struct KeyText {
    pub text: String,
    pub source: String,
}

/// The texts of the server's key files. [`check`] asks for them in the
/// order of these methods and stops at the first refusal, so a text is
/// asked for only when every key before it passed.
pub trait KeyTexts {
    /// Why a text could not be had, such as a file that could not be read.
    type Error;
    /// The access key's text; `None` when the server has no access key.
    fn access(&mut self) -> Result<Option<KeyText>, Self::Error>;
    /// The web key's text; `None` when the web page has no key.
    fn web(&mut self) -> Result<Option<KeyText>, Self::Error>;
    /// The admin key's text; `None` when there is no admin key.
    fn admin(&mut self) -> Result<Option<KeyText>, Self::Error>;
    /// The text of the name file beside the admin key; `None` when there is
    /// none.
    fn admin_names(&mut self) -> Result<Option<String>, Self::Error>;
}

/// The server's keys, as [`check`] accepted them.
pub struct ServerKeys {
    /// Who may use `/mcp`.
    pub admission: Admission,
    /// Who may see the web page.
    pub web_admission: Admission,
    /// The admin key; with none, `/mcp-admin` is off.
    pub admin_key: Option<SharedKey>,
    /// The admin agent's name, when the name file names this admin key.
    pub admin_name: Option<AdminName>,
}

/// Why [`check`] refused the server's keys.
#[derive(Debug, PartialEq, Eq)]
pub enum KeyRefusal<E> {
    /// A text could not be had.
    Text(E),
    /// The access key is not one `check_access_key` accepts.
    AccessKey(SettingError),
    /// The web key is the access key.
    WebKeyReused,
    /// The web key is not one `check_access_key` accepts.
    WebKey(SettingError),
    /// The admin key is the access key or the web key.
    AdminKeyReused,
    /// The admin key is not one `check_access_key` accepts.
    AdminKey(SettingError),
    /// The admin name file is not one `AdminName::bound_in` reads; says why.
    AdminName(String),
}

/// Checks the server's keys, in this order: the access key's form; that
/// the web key differs from the access key, then its form; that the admin
/// key differs from both, then its form; then the admin name. A key is
/// compared with the others before its form is checked, so a reused key
/// is refused as reused before its form is looked at. A name file for a
/// different admin key names nothing.
pub fn check<T: KeyTexts>(texts: &mut T) -> Result<ServerKeys, KeyRefusal<T::Error>> {
    let admission = match texts.access().map_err(KeyRefusal::Text)? {
        None => Admission::Open,
        Some(key) => Admission::SharedKey(
            SharedKey::new(ServerKey::Access, &key.text, key.source)
                .map_err(KeyRefusal::AccessKey)?,
        ),
    };
    let web_admission = match texts.web().map_err(KeyRefusal::Text)? {
        None => Admission::Open,
        Some(key) => {
            if is_one_of(&key.text, &[&admission]) {
                return Err(KeyRefusal::WebKeyReused);
            }
            Admission::SharedKey(
                SharedKey::new(ServerKey::Web, &key.text, key.source)
                    .map_err(KeyRefusal::WebKey)?,
            )
        }
    };
    let (admin_key, admin_name) = match texts.admin().map_err(KeyRefusal::Text)? {
        None => (None, None),
        Some(key) => {
            if is_one_of(&key.text, &[&admission, &web_admission]) {
                return Err(KeyRefusal::AdminKeyReused);
            }
            let shared = SharedKey::new(ServerKey::Admin, &key.text, key.source)
                .map_err(KeyRefusal::AdminKey)?;
            let name = match texts.admin_names().map_err(KeyRefusal::Text)? {
                None => None,
                Some(names) => AdminName::bound_in(&names, &key.text, MAX_ADMIN_NAME_CHARS)
                    .map_err(KeyRefusal::AdminName)?,
            };
            (Some(shared), name)
        }
    };
    Ok(ServerKeys {
        admission,
        web_admission,
        admin_key,
        admin_name,
    })
}

/// Whether the key in `text`, without surrounding whitespace, is the key
/// of any of `others`.
fn is_one_of(text: &str, others: &[&Admission]) -> bool {
    others.iter().any(|other| match other {
        Admission::SharedKey(key) => key.matches(text.trim().as_bytes()),
        Admission::Open | Admission::Bearer => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check_access_key;

    const ACCESS: &str = "0123456789abcdef0123456789abcdef";
    const WEB: &str = "fedcba9876543210fedcba9876543210";
    const ADMIN: &str = "abcdefabcdefabcdefabcdefabcdefab";

    /// Texts from a script, logging which are asked for, in order.
    #[derive(Default)]
    struct Scripted {
        access: Option<&'static str>,
        web: Option<&'static str>,
        admin: Option<&'static str>,
        names: Option<String>,
        /// The text whose read fails, if any.
        fails: Option<&'static str>,
        asked: Vec<&'static str>,
    }

    impl Scripted {
        fn give(
            &mut self,
            which: &'static str,
            text: Option<&str>,
        ) -> Result<Option<KeyText>, String> {
            self.asked.push(which);
            if self.fails == Some(which) {
                return Err(format!("{which} unreadable"));
            }
            Ok(text.map(|text| KeyText {
                text: text.to_string(),
                source: format!("/keys/{which}"),
            }))
        }
    }

    impl KeyTexts for Scripted {
        type Error = String;

        fn access(&mut self) -> Result<Option<KeyText>, String> {
            let text = self.access;
            self.give("access", text)
        }

        fn web(&mut self) -> Result<Option<KeyText>, String> {
            let text = self.web;
            self.give("web", text)
        }

        fn admin(&mut self) -> Result<Option<KeyText>, String> {
            let text = self.admin;
            self.give("admin", text)
        }

        fn admin_names(&mut self) -> Result<Option<String>, String> {
            self.asked.push("names");
            if self.fails == Some("names") {
                return Err("names unreadable".into());
            }
            Ok(self.names.clone())
        }
    }

    fn all_keys() -> Scripted {
        Scripted {
            access: Some(ACCESS),
            web: Some(WEB),
            admin: Some(ADMIN),
            ..Scripted::default()
        }
    }

    fn refusal(texts: &mut Scripted) -> KeyRefusal<String> {
        match check(texts) {
            Ok(_) => panic!("the keys were accepted"),
            Err(refusal) => refusal,
        }
    }

    fn malformed(name: &'static str) -> KeyRefusal<String> {
        let invalid = check_access_key("short").expect_err("a short key");
        assert_eq!(invalid.name, "access_key_file");
        match name {
            "access" => KeyRefusal::AccessKey(invalid),
            "web" => KeyRefusal::WebKey(invalid),
            _ => KeyRefusal::AdminKey(invalid),
        }
    }

    /// kkt's settings reach admission only through `check`, so this is
    /// what keeps `Admission::Bearer` out of kkt: whichever keys are set,
    /// each mode is open or a shared key, and a made-up bearer is never let
    /// in as itself.
    #[test]
    fn no_keys_make_the_bearer_mode() {
        use crate::{Credentials, Decision, Principal};
        let made_up = Credentials::from_authorization(Some(b"Bearer made-up"));
        for set in 0..8u8 {
            let mut texts = Scripted {
                access: (set & 1 != 0).then_some(ACCESS),
                web: (set & 2 != 0).then_some(WEB),
                admin: (set & 4 != 0).then_some(ADMIN),
                ..Scripted::default()
            };
            let keys = check(&mut texts).expect("accepted");
            for admission in [&keys.admission, &keys.web_admission] {
                assert!(
                    matches!(admission, Admission::Open | Admission::SharedKey(_)),
                    "keys {set:03b}"
                );
                assert!(
                    !matches!(
                        admission.decide(&made_up),
                        Decision::Admitted(Principal::Bearer(_))
                    ),
                    "keys {set:03b}"
                );
            }
        }
    }

    #[test]
    fn no_keys_leave_everything_open() {
        let mut texts = Scripted::default();
        let keys = check(&mut texts).expect("accepted");
        assert!(matches!(keys.admission, Admission::Open));
        assert!(matches!(keys.web_admission, Admission::Open));
        assert!(keys.admin_key.is_none());
        assert_eq!(keys.admin_name, None);
        assert_eq!(texts.asked, ["access", "web", "admin"]);
    }

    #[test]
    fn three_different_keys_are_accepted_with_their_sources() {
        let mut texts = all_keys();
        let keys = check(&mut texts).expect("accepted");
        assert_eq!(keys.admission.key_source(), Some("/keys/access"));
        assert_eq!(keys.web_admission.key_source(), Some("/keys/web"));
        let admin = keys.admin_key.expect("an admin key");
        assert_eq!(admin.source(), "/keys/admin");
        assert!(admin.matches(ADMIN.as_bytes()));
        assert_eq!(keys.admin_name, None);
        assert_eq!(texts.asked, ["access", "web", "admin", "names"]);
    }

    #[test]
    fn each_key_admits_its_holder_as_that_key() {
        use crate::{Credentials, Decision, Principal};
        let keys = check(&mut all_keys()).expect("accepted");
        let admin = Admission::SharedKey(keys.admin_key.expect("an admin key"));
        for (admission, text, key) in [
            (&keys.admission, ACCESS, ServerKey::Access),
            (&keys.web_admission, WEB, ServerKey::Web),
            (&admin, ADMIN, ServerKey::Admin),
        ] {
            let presented = Credentials::from_key(text);
            assert_eq!(
                admission.decide(&presented),
                Decision::Admitted(Principal::KeyHolder(key))
            );
        }
    }

    #[test]
    fn a_name_file_for_this_admin_key_names_it() {
        let name = AdminName::new("sentinel", MAX_ADMIN_NAME_CHARS).expect("a name");
        let mut texts = Scripted {
            admin: Some("abcdefabcdefabcdefabcdefabcdefab\n"),
            names: Some(name.bound_to(ADMIN)),
            ..all_keys()
        };
        let keys = check(&mut texts).expect("accepted");
        assert_eq!(keys.admin_name, Some(name));
    }

    #[test]
    fn a_name_file_for_another_admin_key_names_nothing() {
        let name = AdminName::new("sentinel", MAX_ADMIN_NAME_CHARS).expect("a name");
        let mut texts = Scripted {
            names: Some(name.bound_to(WEB)),
            ..all_keys()
        };
        let keys = check(&mut texts).expect("accepted");
        assert!(keys.admin_key.is_some());
        assert_eq!(keys.admin_name, None);
    }

    #[test]
    fn a_bad_name_file_is_refused() {
        let mut texts = Scripted {
            names: Some("sentinel\n".into()),
            ..all_keys()
        };
        assert_eq!(
            refusal(&mut texts),
            KeyRefusal::AdminName(
                "its second line must be `key-sha256: ` and a hash of the admin key".into()
            )
        );
    }

    #[test]
    fn a_malformed_access_key_is_refused_before_the_web_key_is_read() {
        let mut texts = Scripted {
            access: Some("short"),
            fails: Some("web"),
            ..all_keys()
        };
        assert_eq!(refusal(&mut texts), malformed("access"));
        assert_eq!(texts.asked, ["access"]);
    }

    #[test]
    fn a_web_key_that_is_the_access_key_is_refused() {
        let mut texts = Scripted {
            web: Some("  0123456789abcdef0123456789abcdef\n"),
            ..all_keys()
        };
        assert_eq!(refusal(&mut texts), KeyRefusal::WebKeyReused);
        assert_eq!(texts.asked, ["access", "web"]);
    }

    #[test]
    fn a_malformed_web_key_is_refused() {
        let mut texts = Scripted {
            web: Some("short"),
            ..all_keys()
        };
        assert_eq!(refusal(&mut texts), malformed("web"));
        assert_eq!(texts.asked, ["access", "web"]);
    }

    #[test]
    fn an_admin_key_that_is_the_access_key_or_the_web_key_is_refused() {
        for reused in [ACCESS, WEB, "\tfedcba9876543210fedcba9876543210 \n"] {
            let mut texts = Scripted {
                admin: Some(reused),
                ..all_keys()
            };
            assert_eq!(refusal(&mut texts), KeyRefusal::AdminKeyReused);
            assert_eq!(texts.asked, ["access", "web", "admin"]);
        }
    }

    #[test]
    fn an_open_server_lets_any_admin_key_differ() {
        let mut texts = Scripted {
            access: None,
            web: None,
            ..all_keys()
        };
        assert!(check(&mut texts).expect("accepted").admin_key.is_some());
    }

    #[test]
    fn a_malformed_admin_key_is_refused_before_its_name_is_read() {
        let mut texts = Scripted {
            admin: Some("short"),
            fails: Some("names"),
            ..all_keys()
        };
        assert_eq!(refusal(&mut texts), malformed("admin"));
        assert_eq!(texts.asked, ["access", "web", "admin"]);
    }

    #[test]
    fn a_text_that_cannot_be_had_stops_the_check() {
        for (fails, asked) in [
            ("access", &["access"][..]),
            ("web", &["access", "web"]),
            ("admin", &["access", "web", "admin"]),
            ("names", &["access", "web", "admin", "names"]),
        ] {
            let mut texts = Scripted {
                fails: Some(fails),
                ..all_keys()
            };
            assert_eq!(
                refusal(&mut texts),
                KeyRefusal::Text(format!("{fails} unreadable"))
            );
            assert_eq!(texts.asked, asked);
        }
    }
}
