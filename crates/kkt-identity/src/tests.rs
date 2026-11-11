use super::testing::FixedIdentify;
use super::*;
use kkt_api_agent::{Audit, Code, Permission, Permissions};

fn reader() -> Operator {
    Operator::new(Permissions::of(&[Permission::Read]), Audit::WebPage)
}

#[tokio::test]
async fn a_fixed_identity_gives_the_swarm_and_operator_it_was_given() {
    let fixed = FixedIdentify {
        swarm: SwarmId::new("sw_test"),
        avatar: None,
        operator: Some(reader()),
    };
    assert_eq!(
        fixed.newcomer(&Principal::Anyone).await.ok(),
        Some(SwarmId::new("sw_test"))
    );
    let who = fixed
        .operator(&Principal::Anyone)
        .await
        .expect("the operator given");
    assert_eq!(who.swarm, SwarmId::new("sw_test"));
    assert_eq!(who.actor, reader());
}

#[tokio::test]
async fn a_fixed_identity_with_no_operator_refuses_one() {
    let fixed = FixedIdentify {
        swarm: SwarmId::main(),
        avatar: None,
        operator: None,
    };
    let refused = fixed
        .operator(&Principal::Anyone)
        .await
        .expect_err("no operator was given");
    assert_eq!(refused.code, Code::Unauthenticated);
}
