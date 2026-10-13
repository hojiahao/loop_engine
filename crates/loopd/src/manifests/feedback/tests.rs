use super::*;
use crate::manifests::numerical::tests::Case;
use loop_core::factor;

fn policy() -> FeedbackPolicy {
    FeedbackPolicy {
        schema: "loop.run-feedback/v1".to_owned(),
        id: "feedback.window".to_owned(),
        revision: "1".to_owned(),
        proposal_schedule: vec![
            Proposal::Discovery,
            Proposal::Perturbation,
            Proposal::Discovery,
        ],
        window_path: vec![1],
        eligible_windows: vec![2, 5, 10],
        random_seed: format!("sha256:{}", "03".repeat(32)),
        max_observations: 16,
        max_bytes: 16384,
    }
}

async fn freeze(case: &Case, policy: &FeedbackPolicy) -> FrozenFeedback {
    let object = case.fixture.json(policy);
    case.resolver
        .feedback(
            &wire::PolicyReference {
                policy_id: Some(wire::PolicyId {
                    value: policy.id.clone(),
                }),
                revision: policy.revision.clone(),
                sha256: Some(model::digest(object.digest().unwrap())),
            },
            &object,
        )
        .await
        .unwrap()
}

#[test]
fn policy_bounds() {
    let valid = policy();
    valid.validate().unwrap();
    let mut invalid = valid.clone();
    invalid.eligible_windows = vec![2, 2, 5];
    assert!(invalid.validate().is_err());
    invalid = valid.clone();
    invalid.eligible_windows = vec![10, 5];
    assert!(invalid.validate().is_err());
    invalid = valid.clone();
    invalid.proposal_schedule[0] = Proposal::Perturbation;
    assert!(invalid.validate().is_err());
    invalid = valid.clone();
    invalid.max_bytes = 16385;
    assert!(invalid.validate().is_err());
    invalid = valid;
    invalid.max_observations = 17;
    assert!(invalid.validate().is_err());
}

#[tokio::test]
async fn policy_guard() {
    let case = Case::new();
    let frozen = freeze(&case, &policy()).await;
    assert_eq!(frozen.proposal(2).unwrap(), Proposal::Perturbation);
    assert!(frozen.proposal(0).is_err());
    assert!(frozen.proposal(4).is_err());
    let path = case.fixture.path(frozen.reference());
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(path, bytes).unwrap();
    assert!(frozen.check().is_err());
}

#[tokio::test]
async fn policy_identity() {
    let case = Case::new();
    let document = policy();
    let object = case.fixture.json(&document);
    let wrong = wire::PolicyReference {
        policy_id: Some(wire::PolicyId { value: document.id }),
        revision: "1".to_owned(),
        sha256: Some(model::digest([7; 32])),
    };
    assert!(case.resolver.feedback(&wrong, &object).await.is_err());
}

#[tokio::test]
async fn family_identity() {
    let case = Case::new();
    let numerical = case.prepare().await.unwrap();
    let policy = freeze(&case, &policy()).await;
    let family = numerical.family(&ast(5), &policy).unwrap().unwrap();
    assert_eq!(family.space().candidates.len(), 3);
    for candidate in &family.space().candidates {
        let independently = numerical
            .evaluation(&ast(candidate.window), wire::JobBudget::default())
            .unwrap();
        assert_eq!(
            candidate.factor_spec_id,
            independently.factor.unwrap().factor_spec_id
        );
        assert_eq!(family.ast(candidate).unwrap(), &ast(candidate.window));
    }
    // The initial proposal's actual window is excluded from family identity.
    assert_eq!(
        family.digest(),
        numerical
            .family(&ast(2), &policy)
            .unwrap()
            .unwrap()
            .digest()
    );
}

#[tokio::test]
async fn family_seed() {
    let case = Case::new();
    let numerical = case.prepare().await.unwrap();
    let one = freeze(&case, &policy()).await;
    let mut changed = policy();
    changed.random_seed = format!("sha256:{}", "04".repeat(32));
    let two = freeze(&case, &changed).await;
    let before = numerical.family(&ast(5), &one).unwrap().unwrap();
    let after = numerical.family(&ast(5), &two).unwrap().unwrap();
    assert_ne!(before.digest(), after.digest());
    assert_eq!(before.space().candidates, after.space().candidates);
}

#[tokio::test]
async fn family_path() {
    let case = Case::new();
    let numerical = case.prepare().await.unwrap();
    let mut invalid = policy();
    invalid.window_path = vec![0];
    let frozen = freeze(&case, &invalid).await;
    assert!(numerical.family(&ast(5), &frozen).unwrap().is_none());
    let mut mismatched = ast(5);
    mismatched.canonical_json = ast(2).canonical_json;
    assert!(numerical.family(&mismatched, &frozen).is_err());
}

#[tokio::test]
async fn feedback_replay() {
    let case = Case::new();
    let policy = freeze(&case, &policy()).await;
    let written = case
        .resolver
        .feedback_context(&policy, "run.feedback", 1, vec![], false)
        .await
        .unwrap();
    let recovered = case
        .resolver
        .feedback_context(&policy, "run.feedback", 1, vec![], true)
        .await
        .unwrap();
    assert_eq!(written.reference, recovered.reference);
    assert_eq!(written.text, recovered.text);
    std::fs::remove_file(case.fixture.path(&written.reference)).unwrap();
    assert!(
        case.resolver
            .feedback_context(&policy, "run.feedback", 1, vec![], true)
            .await
            .is_err()
    );
    assert!(written.check().is_err());
}

fn ast(window: u32) -> wire::CanonicalFactorAst {
    let registry = factor::us_equities::registry().unwrap();
    let expression = factor::FactorExpr::Call(factor::OperatorCall::new(
        factor::OperatorRef::new(
            factor::Identifier::new("ma").unwrap(),
            factor::PositiveInteger::new("2").unwrap(),
        ),
        vec![
            factor::FactorExpr::Field(factor::FieldRef::new(
                factor::Identifier::new("market.close").unwrap(),
            )),
            factor::FactorExpr::Literal(factor::Literal::Decimal(
                factor::CanonicalDecimal::new(window.to_string()).unwrap(),
            )),
            factor::FactorExpr::Literal(factor::Literal::Decimal(
                factor::CanonicalDecimal::new("2").unwrap(),
            )),
        ],
    ));
    wire::CanonicalFactorAst {
        expression_id: Some(wire::FactorExpressionId {
            value: factor::expression_id(
                &expression,
                &registry,
                factor::ValidationLimits::default(),
            )
            .unwrap()
            .to_string(),
        }),
        ast: Some(wire::FactorAst {
            schema_version: 1,
            root: Some(wire::FactorAstNode {
                node: Some(wire::factor_ast_node::Node::Call(wire::OperatorCall {
                    operator: Some(wire::OperatorReference {
                        operator: "ma".to_owned(),
                        operator_version: "2".to_owned(),
                    }),
                    arguments: vec![
                        wire::FactorAstNode {
                            node: Some(wire::factor_ast_node::Node::Field(wire::FieldReference {
                                field: "market.close".to_owned(),
                            })),
                        },
                        wire::FactorAstNode {
                            node: Some(wire::factor_ast_node::Node::Literal(wire::FactorLiteral {
                                value: Some(wire::factor_literal::Value::Decimal(
                                    wire::ExactDecimal {
                                        value: window.to_string(),
                                    },
                                )),
                            })),
                        },
                        wire::FactorAstNode {
                            node: Some(wire::factor_ast_node::Node::Literal(wire::FactorLiteral {
                                value: Some(wire::factor_literal::Value::Decimal(
                                    wire::ExactDecimal {
                                        value: "2".to_owned(),
                                    },
                                )),
                            })),
                        },
                    ],
                })),
            }),
        }),
        canonicalization_profile: "loop.factor-ast/v1".to_owned(),
        canonical_json: factor::canonical_expression_bytes(
            &expression,
            &registry,
            factor::ValidationLimits::default(),
        )
        .unwrap(),
    }
}
