//! How notices name the operator, by default: the words [`OperatorWording`]
//! asks for, from where the operator's request came from.

use super::*;

/// How notices name the operator, by default: "The operator" for the web
/// page, and "The operator (via `<name>`)" for an admin agent under its admin
/// key's name. An admin key with no name is worded like the web page; it
/// never posts a notice, since the admin endpoint never gives it
/// [`Permission::Change`]. kkt-rules' own [`OperatorWording`]: the program
/// passes it in, and another product may pass its own.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultWording;

impl OperatorWording for DefaultWording {
    fn operator(&self, audit: &Audit) -> String {
        match audit {
            Audit::WebPage | Audit::AdminEndpoint { key_name: None } => "The operator".into(),
            Audit::AdminEndpoint {
                key_name: Some(name),
            } => format!("The operator (via {name})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kkt_names::AdminName;

    #[test]
    fn notices_name_the_operator_as_kkt_always_has() {
        let name = AdminName::new("sentinel", 40).expect("a name");
        let words = |audit: Audit| DefaultWording.operator(&audit);
        assert_eq!(words(Audit::WebPage), "The operator");
        assert_eq!(
            words(Audit::AdminEndpoint {
                key_name: Some(name.to_string())
            }),
            "The operator (via admin-sentinel)"
        );
        // An unnamed admin key changes nothing, so posts no notice; were it
        // named in one, it would be the operator, unadorned.
        assert_eq!(
            words(Audit::AdminEndpoint { key_name: None }),
            "The operator"
        );
    }
}
