use super::*;

fn sentinel() -> AdminName {
    AdminName::new("sentinel", kkt_config::MAX_ADMIN_NAME_CHARS).expect("a name")
}

const ALL: [Permission; 3] = [
    Permission::Read,
    Permission::Change,
    Permission::IssueRecoveryCodes,
];

/// The permissions `operator` holds, in `ALL`'s order.
fn held(operator: &Operator) -> Vec<Permission> {
    ALL.into_iter()
        .filter(|p| operator.permissions.has(*p))
        .collect()
}

#[tokio::test]
async fn each_principal_is_the_operator_its_key_makes_in_the_one_swarm() {
    let named = KeyMode::new(Some(sentinel()));
    let unnamed = KeyMode::new(None);
    let cases: [(&KeyMode, Principal, Vec<Permission>, Audit); 5] = [
        (
            &named,
            Principal::KeyHolder(ServerKey::Admin),
            vec![Permission::Read, Permission::Change],
            Audit::AdminEndpoint {
                key_name: Some(sentinel()),
            },
        ),
        (
            &unnamed,
            Principal::KeyHolder(ServerKey::Admin),
            vec![Permission::Read],
            Audit::AdminEndpoint { key_name: None },
        ),
        (
            &named,
            Principal::KeyHolder(ServerKey::Web),
            ALL.to_vec(),
            Audit::WebPage,
        ),
        (
            &unnamed,
            Principal::KeyHolder(ServerKey::Web),
            ALL.to_vec(),
            Audit::WebPage,
        ),
        (&named, Principal::Anyone, ALL.to_vec(), Audit::WebPage),
    ];
    for (mode, principal, permissions, audit) in cases {
        let who = mode.operator(&principal).await.expect("an operator");
        assert_eq!(who.swarm, SwarmId::main(), "{principal:?}");
        assert_eq!(held(&who.actor), permissions, "{principal:?}");
        assert_eq!(who.actor.audit, audit, "{principal:?}");
    }
}

#[tokio::test]
async fn the_access_key_makes_no_operator() {
    for mode in [KeyMode::new(Some(sentinel())), KeyMode::new(None)] {
        let refused = mode
            .operator(&Principal::KeyHolder(ServerKey::Access))
            .await
            .expect_err("not an operator");
        assert_eq!(refused.code, Code::Unauthenticated);
    }
}

#[tokio::test]
async fn newcomers_are_made_in_the_one_swarm_whoever_asks() {
    let mode = KeyMode::new(None);
    for principal in [
        Principal::Anyone,
        Principal::KeyHolder(ServerKey::Access),
        Principal::KeyHolder(ServerKey::Web),
        Principal::KeyHolder(ServerKey::Admin),
    ] {
        assert_eq!(
            mode.newcomer(&principal).await.expect("allowed"),
            SwarmId::main()
        );
    }
}

#[test]
fn a_named_admin_key_is_named_as_the_author_and_an_unnamed_one_is_not() {
    assert_eq!(
        KeyMode::admin_operator(Some(sentinel()))
            .audit
            .author()
            .as_deref(),
        Some("admin-sentinel")
    );
    assert_eq!(KeyMode::admin_operator(None).audit.author(), None);
}
