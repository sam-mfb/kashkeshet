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

/// Rules over a new database, in `sw_main`.
async fn rules() -> Rules {
    use kkt_clock::Time;
    use kkt_clock::testing::FixedClock;
    use kkt_names::testing::SeqEntropy;
    use std::sync::Arc;

    struct Plain;
    impl rules::OperatorWording for Plain {
        fn operator(&self, _audit: &Audit) -> String {
            "The operator".into()
        }
    }

    let db = kkt_store::Db::new(
        kkt_db_sqlite::MemoryPlace::new()
            .expect("an in-memory database")
            .open(&[kkt_store::schema()])
            .expect("the core's tables"),
    );
    let run_id = db
        .write(|w| {
            w.insert_server_run(&kkt_store::NewServerRunRecord {
                started_at: "2026-11-10T00:00:00Z".into(),
                version: "test".into(),
                git_commit: "test".into(),
                settings: "test".into(),
                instructions_sha256: "test".into(),
            })
        })
        .await
        .expect("a server run");
    Rules::new(rules::Deps {
        db,
        swarm: SwarmId::main(),
        events: kkt_events::Events::new(),
        clock: Arc::new(FixedClock::new(
            Time::from_parts(2026, 11, 10, 0, 0, 0).unwrap(),
        )),
        entropy: Arc::new(SeqEntropy::new(1)),
        run_id,
        settings: kkt_rules::Settings::DEFAULT,
        wording: Arc::new(Plain),
    })
}

#[tokio::test]
async fn key_mode_refuses_a_bearer_everything() {
    let rules = rules().await;
    let registered = rules.register().await.expect("register");
    let mode = KeyMode::new(Some(sentinel()));
    let bearer = Principal::Bearer(kkt_admission::BearerToken::new("made-up"));

    // The same token is good from a caller admitted by a key.
    let admitted = mode
        .avatar(&rules, &Principal::Anyone, Some(&registered.token))
        .await
        .expect("the avatar");
    assert_eq!(admitted.actor.handle, registered.handle);

    let refusals = [
        mode.avatar(&rules, &bearer, Some(&registered.token))
            .await
            .expect_err("no avatar")
            .code,
        mode.newcomer(&bearer).await.expect_err("no newcomer").code,
        mode.operator(&bearer).await.expect_err("no operator").code,
    ];
    assert_eq!(refusals, [Code::Unauthenticated; 3]);
}
