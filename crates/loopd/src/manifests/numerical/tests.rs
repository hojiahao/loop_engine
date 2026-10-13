use super::*;
use crate::manifests::tests::fixture::Fixture;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;

struct Case {
    fixture: Fixture,
    recipe: Recipe,
    dataset: wire::DevelopmentDatasetReference,
    resolver: NumericalResolver,
}

impl Case {
    fn new() -> Self {
        let mut fixture = Fixture::new();
        std::fs::set_permissions(&fixture.root, std::fs::Permissions::from_mode(0o700)).unwrap();
        fixture.registry = Arc::new(factor::us_equities::registry().unwrap());
        fixture.context.registry = put(&fixture, &fixture.registry.canonical_bytes());
        fixture.context.family = None;
        let mut configuration: model::Configuration =
            read(&fixture, &fixture.context.configuration);
        configuration.backtest_engine_version = "factor-evaluator.2".to_owned();
        fixture.context.configuration = fixture.json(&configuration);
        let mut dataset: model::Dataset = read(&fixture, &fixture.context.data);
        let tape = fixture.json(&serde_json::json!({"schema":"loop.execution-tape/v1"}));
        let schema = fixture.json(&model::SchemaDocument {
            schema: "loop.artifact-schema/v1".to_owned(),
            name: "loop.execution_tape".to_owned(),
            version: 1,
            media_type: "application/json".to_owned(),
            columns: vec![],
        });
        let artifact = &mut dataset.snapshots[0].artifacts[0];
        artifact.object = tape.clone();
        artifact.media_type = "application/json".to_owned();
        artifact.schema.name = "loop.execution_tape".to_owned();
        artifact.schema.document = schema;
        fixture.context.data = fixture.json(&dataset);
        let recipe = Recipe {
            schema: "loop.run-numerical/v1".to_owned(),
            id: "recipe.numerical".to_owned(),
            revision: "1".to_owned(),
            evaluation_context: fixture.json(&fixture.context),
            direction: factor::FactorDirection::HigherIsBetter,
            deterministic_seed: format!("sha256:{}", "01".repeat(32)),
            execution_tape: tape,
            policies: POLICY_ROLES
                .into_iter()
                .map(|role| {
                    (
                        role.to_owned(),
                        format!("policy.{}", role.strip_suffix("_policy").unwrap()),
                    )
                })
                .collect(),
            hypothesis: "The frozen observation ranks future returns.".to_owned(),
            required_observations: vec![
                "Daily closing prices available at decision time.".to_owned(),
            ],
            falsification_criteria: vec![
                "Insufficient coverage under the frozen evaluation policy.".to_owned(),
            ],
        };
        let resolver = NumericalResolver::open(&fixture.root, &fixture.root).unwrap();
        let reference = dataset.reference(&fixture.context.data).unwrap();
        Self {
            fixture,
            recipe,
            dataset: reference,
            resolver,
        }
    }

    fn pin(&self) -> (wire::PolicyReference, ObjectRef) {
        let object = self.fixture.json(&self.recipe);
        let policy = wire::PolicyReference {
            policy_id: Some(wire::PolicyId {
                value: self.recipe.id.clone(),
            }),
            revision: self.recipe.revision.clone(),
            sha256: Some(model::digest(object.digest().unwrap())),
        };
        (policy, object)
    }

    async fn prepare(&self) -> StoreResult<FrozenNumerical> {
        let (policy, object) = self.pin();
        self.resolver.prepare(&policy, &object, &self.dataset).await
    }
}

#[tokio::test]
async fn registry_identity() {
    let case = Case::new();
    let frozen = case.prepare().await.unwrap();
    let input = frozen
        .evaluation(&ast(), wire::JobBudget::default())
        .unwrap();
    let factor = input.factor.unwrap();
    assert_eq!(
        factor.operator_registry_sha256.unwrap().value,
        case.fixture.registry.identity().as_bytes()
    );
    assert_ne!(
        case.fixture.context.registry.digest().unwrap(),
        *case.fixture.registry.identity().as_bytes()
    );
    assert_eq!(input.deterministic_seed.unwrap().value, vec![1; 32]);
    assert_eq!(input.dataset.unwrap(), case.dataset);
}

#[tokio::test]
async fn stable_factor() {
    let case = Case::new();
    let first = case
        .prepare()
        .await
        .unwrap()
        .evaluation(&ast(), wire::JobBudget::default())
        .unwrap();
    let second = case
        .prepare()
        .await
        .unwrap()
        .evaluation(&ast(), wire::JobBudget::default())
        .unwrap();
    assert_eq!(first, second);
    loop_protocol::job::validate_factor_identity(first.factor.as_ref().unwrap()).unwrap();
}

#[tokio::test]
async fn recipe_binding() {
    let case = Case::new();
    let (mut policy, object) = case.pin();
    policy.sha256.as_mut().unwrap().value[0] ^= 1;
    assert!(
        case.resolver
            .prepare(&policy, &object, &case.dataset)
            .await
            .is_err()
    );
    let (mut policy, object) = case.pin();
    policy.revision = "2".to_owned();
    assert!(
        case.resolver
            .prepare(&policy, &object, &case.dataset)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn dataset_binding() {
    let case = Case::new();
    let (policy, object) = case.pin();
    let mut dataset = case.dataset.clone();
    dataset.manifest_sha256.as_mut().unwrap().value[0] ^= 1;
    assert!(
        case.resolver
            .prepare(&policy, &object, &dataset)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn policy_roles() {
    let mut case = Case::new();
    case.recipe
        .policies
        .insert("data_policy".to_owned(), "policy.universe".to_owned());
    assert!(case.prepare().await.is_err());
    case.recipe
        .policies
        .insert("data_policy".to_owned(), "policy.unknown".to_owned());
    assert!(case.prepare().await.is_err());
}

#[tokio::test]
async fn tape_binding() {
    let mut case = Case::new();
    case.recipe.execution_tape = case.recipe.evaluation_context.clone();
    assert!(case.prepare().await.is_err());
}

#[tokio::test]
async fn seed_format() {
    let mut case = Case::new();
    case.recipe.deterministic_seed = format!("sha256:{}", "01".repeat(31));
    assert!(case.prepare().await.is_err());
}

#[tokio::test]
async fn knowledge_boundary() {
    let mut case = Case::new();
    let mut dataset: model::Dataset = read(&case.fixture, &case.fixture.context.data);
    dataset.snapshots[0].known_through_ms = 1_452_297_600_000;
    case.fixture.context.data = case.fixture.json(&dataset);
    case.recipe.evaluation_context = case.fixture.json(&case.fixture.context);
    case.dataset = dataset.reference(&case.fixture.context.data).unwrap();
    assert!(case.prepare().await.is_err());
}

#[tokio::test]
async fn declared_policies() {
    let mut case = Case::new();
    let mut configuration: model::Configuration =
        read(&case.fixture, &case.fixture.context.configuration);
    for (index, policy) in configuration.policies.iter_mut().enumerate() {
        let prior = policy.policy_id.clone();
        policy.policy_id = format!("opaque.p{index}");
        let mut document: model::PolicyDocument = read(&case.fixture, &policy.document);
        document.policy_id = policy.policy_id.clone();
        policy.document = case.fixture.json(&document);
        for id in case.recipe.policies.values_mut() {
            if id == &prior {
                *id = policy.policy_id.clone();
            }
        }
    }
    configuration
        .policies
        .sort_by(|left, right| left.policy_id.cmp(&right.policy_id));
    case.fixture.context.configuration = case.fixture.json(&configuration);
    case.recipe.evaluation_context = case.fixture.json(&case.fixture.context);
    let frozen = case.prepare().await.unwrap();
    let input = frozen
        .evaluation(&ast(), wire::JobBudget::default())
        .unwrap();
    assert!(
        input
            .factor
            .unwrap()
            .frozen_policy
            .unwrap()
            .universe_policy
            .unwrap()
            .policy_id
            .unwrap()
            .value
            .starts_with("opaque.")
    );
}

#[tokio::test]
async fn holdout_denied() {
    let mut case = Case::new();
    let mut dataset: model::Dataset = read(&case.fixture, &case.fixture.context.data);
    dataset.sample.role = model::SampleRole::FirstLockedConfirmation;
    dataset.sample.start = "2021-01-01".to_owned();
    dataset.sample.end = "2024-12-31".to_owned();
    case.fixture.context.data = case.fixture.json(&dataset);
    case.recipe.evaluation_context = case.fixture.json(&case.fixture.context);
    case.dataset = dataset.reference(&case.fixture.context.data).unwrap();
    assert!(case.prepare().await.is_err());
}

#[tokio::test]
async fn source_guards() {
    let case = Case::new();
    let frozen = case.prepare().await.unwrap();
    let source: model::FileSet = read(&case.fixture, &case.fixture.context.source);
    let path = case.fixture.path(&source.files[0].object);
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[0] ^= 1;
    std::fs::write(path, bytes).unwrap();
    assert!(frozen.check().is_err());
    assert!(
        frozen
            .evaluation(&ast(), wire::JobBudget::default())
            .is_err()
    );
    assert!(case.prepare().await.is_err());
}

#[tokio::test]
async fn replaced_source() {
    let case = Case::new();
    let frozen = case.prepare().await.unwrap();
    let source: model::FileSet = read(&case.fixture, &case.fixture.context.source);
    let path = case.fixture.path(&source.files[0].object);
    let original = std::fs::read(&path).unwrap();
    let mut changed = original.clone();
    changed[0] ^= 1;
    let mut replacement = tempfile::NamedTempFile::new_in(&case.fixture.root).unwrap();
    replacement.write_all(&changed).unwrap();
    replacement.persist(&path).unwrap();
    assert!(frozen.check().is_err());
    assert!(case.prepare().await.is_err());

    // Restoring the content permits a fresh proof, never revival of the old
    // descriptor retained across a request's final commit boundary.
    std::fs::write(path, original).unwrap();
    assert!(frozen.check().is_err());
    assert!(case.prepare().await.is_ok());
}

#[tokio::test]
async fn missing_source() {
    let case = Case::new();
    let frozen = case.prepare().await.unwrap();
    let source: model::FileSet = read(&case.fixture, &case.fixture.context.source);
    std::fs::remove_file(case.fixture.path(&source.files[0].object)).unwrap();
    assert!(frozen.check().is_err());
    assert!(case.prepare().await.is_err());
}

#[tokio::test]
async fn root_replacement() {
    let case = Case::new();
    let (policy, object) = case.pin();
    case.resolver
        .prepare(&policy, &object, &case.dataset)
        .await
        .unwrap();
    std::fs::rename(
        &case.fixture.root,
        case.fixture.directory.path().join("old-objects"),
    )
    .unwrap();
    std::fs::create_dir(&case.fixture.root).unwrap();
    std::fs::set_permissions(&case.fixture.root, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(
        case.resolver.prepare(&policy, &object, &case.dataset).await,
        Err(StoreError::Corrupt("numerical publication root changed"))
    ));
}

#[tokio::test]
async fn root_permissions() {
    let case = Case::new();
    let (policy, object) = case.pin();
    case.resolver
        .prepare(&policy, &object, &case.dataset)
        .await
        .unwrap();
    std::fs::set_permissions(&case.fixture.root, std::fs::Permissions::from_mode(0o720)).unwrap();
    assert!(matches!(
        case.resolver.prepare(&policy, &object, &case.dataset).await,
        Err(StoreError::Corrupt("numerical publication root changed"))
    ));
}

#[tokio::test]
async fn ast_binding() {
    let case = Case::new();
    let frozen = case.prepare().await.unwrap();
    let mut expression = ast();
    expression.ast.as_mut().unwrap().root.as_mut().unwrap().node =
        Some(wire::factor_ast_node::Node::Field(wire::FieldReference {
            field: "market.volume".to_owned(),
        }));
    assert!(
        frozen
            .evaluation(&expression, wire::JobBudget::default())
            .is_err()
    );
}

#[tokio::test]
async fn replay_missing() {
    let case = Case::new();
    let mut files = vec![];
    assert!(
        case.resolver
            .publish(b"missing", true, &mut files)
            .await
            .is_err()
    );
    let object = case
        .resolver
        .publish(b"created", false, &mut files)
        .await
        .unwrap();
    case.resolver
        .publish(b"created", true, &mut files)
        .await
        .unwrap();
    std::fs::remove_file(case.fixture.path(&object)).unwrap();
    assert!(
        case.resolver
            .publish(b"created", true, &mut files)
            .await
            .is_err()
    );
    assert!(!case.fixture.path(&object).exists());
}

#[tokio::test]
async fn publication_corruption() {
    let case = Case::new();
    let mut files = vec![];
    let object = case
        .resolver
        .publish(b"created", false, &mut files)
        .await
        .unwrap();
    std::fs::write(case.fixture.path(&object), b"changed").unwrap();
    assert!(
        case.resolver
            .publish(b"created", false, &mut files)
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(case.fixture.path(&object)).unwrap(),
        b"changed"
    );
}

fn ast() -> wire::CanonicalFactorAst {
    let registry = factor::us_equities::registry().unwrap();
    let expression = factor::FactorExpr::Field(factor::FieldRef::new(
        factor::Identifier::new("market.close").unwrap(),
    ));
    let limits = ValidationLimits::default();
    wire::CanonicalFactorAst {
        expression_id: Some(wire::FactorExpressionId {
            value: factor::expression_id(&expression, &registry, limits)
                .unwrap()
                .to_string(),
        }),
        ast: Some(wire::FactorAst {
            schema_version: 1,
            root: Some(wire::FactorAstNode {
                node: Some(wire::factor_ast_node::Node::Field(wire::FieldReference {
                    field: "market.close".to_owned(),
                })),
            }),
        }),
        canonicalization_profile: "loop.factor-ast/v1".to_owned(),
        canonical_json: factor::canonical_expression_bytes(&expression, &registry, limits).unwrap(),
    }
}

fn read<T: serde::de::DeserializeOwned>(fixture: &Fixture, reference: &ObjectRef) -> T {
    serde_json::from_slice(&std::fs::read(fixture.path(reference)).unwrap()).unwrap()
}

fn put(fixture: &Fixture, bytes: &[u8]) -> ObjectRef {
    let object = ObjectRef {
        sha256: format!("sha256:{:x}", Sha256::digest(bytes)),
        byte_size: bytes.len() as u64,
    };
    std::fs::write(fixture.path(&object), bytes).unwrap();
    object
}
