use super::*;

fn identifier(value: &str) -> Identifier {
    Identifier::new(value).expect("test identifier must be valid")
}

fn positive(value: u64) -> PositiveInteger {
    PositiveInteger::from_u64(value).expect("test value must be positive")
}

fn operator(name: &str, version: u64) -> OperatorRef {
    OperatorRef::new(identifier(name), positive(version))
}

fn field(name: &str) -> FactorExpr {
    FactorExpr::Field(FieldRef::new(identifier(name)))
}

fn decimal(value: &str) -> FactorExpr {
    FactorExpr::Literal(Literal::Decimal(
        CanonicalDecimal::new(value).expect("test decimal must be canonical"),
    ))
}

fn call(name: &str, version: u64, arguments: Vec<FactorExpr>) -> FactorExpr {
    FactorExpr::Call(OperatorCall::new(operator(name, version), arguments))
}

fn policy(name: &str, revision: u64, fill: u8) -> PolicyRef {
    PolicyRef::new(
        PolicyId::new(name).expect("test policy ID must be valid"),
        positive(revision),
        [fill; 32],
    )
}

fn semantic_contract(name: &str, version: u64) -> OperatorSemanticContract {
    let (null_policy, window_policy, tie_policy, alignment_policy, numeric_policy) = match name {
        "rolling.mean" => (
            NullPolicy::IgnoreMissing,
            WindowPolicy::TrailingArgument2FullWindowRightInclusiveConstantPreserve,
            TiePolicy::NotApplicable,
            AlignmentPolicy::UnaryPreserveTimestampAndSecurity,
            NumericPolicy::Binary64NonFiniteToMissing,
        ),
        "rank.cross_section" => (
            NullPolicy::PreserveTargetIgnorePeers,
            WindowPolicy::NotApplicable,
            TiePolicy::Argument2AverageOrDenseValidCountConstantMidpoint,
            AlignmentPolicy::UnaryPreserveTimestampAndSecurity,
            NumericPolicy::OrdinalUnitInterval,
        ),
        "decimal.identity" => (
            NullPolicy::NotApplicable,
            WindowPolicy::NotApplicable,
            TiePolicy::NotApplicable,
            AlignmentPolicy::NotApplicable,
            NumericPolicy::ExactDecimal,
        ),
        "math.abs" => (
            NullPolicy::Propagate,
            WindowPolicy::NotApplicable,
            TiePolicy::NotApplicable,
            AlignmentPolicy::UnaryPreserveTimestampAndSecurity,
            NumericPolicy::Binary64NonFiniteToMissing,
        ),
        _ => (
            NullPolicy::Propagate,
            WindowPolicy::NotApplicable,
            TiePolicy::NotApplicable,
            AlignmentPolicy::StrictTimestampAndSecurity,
            NumericPolicy::Binary64NonFiniteToMissing,
        ),
    };
    OperatorSemanticContract::new(
        identifier(name),
        positive(version),
        null_policy,
        window_policy,
        tie_policy,
        alignment_policy,
        numeric_policy,
    )
}

fn register_builder(
    entries: &[(&str, u64, OperatorPolicy)],
) -> Result<
    (
        OperatorRegistryBuilder,
        BTreeMap<SemanticContractId, Vec<u8>>,
    ),
    RegistryError,
> {
    let mut registry = OperatorRegistryBuilder::new();
    let mut contracts = BTreeMap::new();
    for name in [
        "market.close",
        "market.high",
        "market.low",
        "market.open",
        "market.volume",
    ] {
        registry.register_field(identifier(name), ValueType::Series)?;
    }
    registry.register_enum(
        identifier("rank.method"),
        [identifier("average"), identifier("dense")],
    )?;
    for (name, version, policy) in entries {
        let contract = semantic_contract(name, *version);
        let semantic_contract_sha256 = contract.identity();
        contracts.insert(semantic_contract_sha256, contract.canonical_bytes());
        let definition = match *name {
            "rolling.mean" => OperatorDefinition::fixed(
                operator(name, *version),
                vec![
                    ArgumentDefinition::series(),
                    ArgumentDefinition::decimal_literal(
                        DecimalConstraints::new(
                            3,
                            0,
                            CanonicalDecimal::new("2").unwrap(),
                            CanonicalDecimal::new("252").unwrap(),
                        )
                        .unwrap(),
                    ),
                ],
                ValueType::Series,
                *policy,
                semantic_contract_sha256,
            )?,
            "arithmetic.add" => OperatorDefinition::variadic(
                operator(name, *version),
                vec![],
                ArgumentDefinition::series(),
                2,
                ValidationLimits::HARD_MAX_ARGUMENTS,
                ValueType::Series,
                *policy,
                semantic_contract_sha256,
            )?,
            "arithmetic.subtract" => OperatorDefinition::fixed(
                operator(name, *version),
                vec![ArgumentDefinition::series(), ArgumentDefinition::series()],
                ValueType::Series,
                *policy,
                semantic_contract_sha256,
            )?,
            "rank.cross_section" => OperatorDefinition::fixed(
                operator(name, *version),
                vec![
                    ArgumentDefinition::series(),
                    ArgumentDefinition::enumeration_literal(identifier("rank.method")),
                ],
                ValueType::Series,
                *policy,
                semantic_contract_sha256,
            )?,
            "math.abs" => OperatorDefinition::fixed(
                operator(name, *version),
                vec![ArgumentDefinition::series()],
                ValueType::Series,
                *policy,
                semantic_contract_sha256,
            )?,
            _ => unreachable!("test registry only contains declared fixtures"),
        };
        registry.register_operator(definition)?;
    }
    Ok((registry, contracts))
}

fn register(
    entries: &[(&str, u64, OperatorPolicy)],
) -> Result<OperatorPolicyRegistry, RegistryError> {
    let (builder, contracts) = register_builder(entries)?;
    builder.build(&contracts)
}

fn factor_spec_draft(
    expression_id: ExpressionId,
    registry: &OperatorPolicyRegistry,
    direction: FactorDirection,
) -> FactorSpecDraft {
    FactorSpecDraft::new(
        expression_id,
        *registry.identity().as_bytes(),
        direction,
        policy("us_common_stock", 1, 0x11),
        policy("pit_market_v1", 1, 0x21),
        policy("xnys_xnas", 1, 0x31),
        policy("cross_section_v1", 1, 0x41),
        policy("industry_size_beta", 1, 0x51),
        policy("decile_long_short", 1, 0x61),
        policy("next_tradable_open", 1, 0x71),
        policy("us_equities_cost_v1", 1, 0x81),
        policy("factor_admission_v1", 1, 0x91),
    )
}

fn bind_test_spec(
    expression: &FactorExpr,
    registry: &OperatorPolicyRegistry,
    direction: FactorDirection,
) -> FactorSpec {
    let canonical =
        canonical_expression_bytes(expression, registry, ValidationLimits::default()).unwrap();
    let expected = expression_id(expression, registry, ValidationLimits::default()).unwrap();
    bind_test_draft(
        factor_spec_draft(expected, registry, direction),
        &canonical,
        registry,
    )
}

fn bind_test_draft(
    draft: FactorSpecDraft,
    canonical_expression: &[u8],
    registry: &OperatorPolicyRegistry,
) -> FactorSpec {
    bind_factor_spec(
        draft,
        canonical_expression,
        registry,
        ValidationLimits::default(),
    )
    .unwrap()
}

#[test]
fn identifiers_enforce_dot_qualified_ascii_grammar() {
    for valid in [
        "a",
        "market.close",
        "rolling.mean_v2",
        "rank.method.average",
    ] {
        assert_eq!(identifier(valid).as_str(), valid);
    }

    for invalid in [
        "",
        "Market.close",
        "2close",
        "market.2close",
        "market..close",
        ".close",
        "close.",
        "close-price",
        "market/caf\u{00e9}",
    ] {
        assert!(Identifier::new(invalid).is_err(), "accepted {invalid:?}");
    }
    assert!(Identifier::new("a".repeat(129)).is_err());
}

#[test]
fn policy_ids_use_the_separate_declared_grammar() {
    for valid in ["a", "us_common_stock", "policy.v1", "policy-v1"] {
        assert_eq!(PolicyId::new(valid).unwrap().as_str(), valid);
    }
    for invalid in ["", "Policy", "1policy", "policy/value", "\u{653f}\u{7b56}"] {
        assert!(PolicyId::new(invalid).is_err(), "accepted {invalid:?}");
    }
    assert!(PolicyId::new("a".repeat(129)).is_err());
}

#[test]
fn serde_cannot_bypass_validated_scalar_constructors() {
    assert!(serde_json::from_str::<Identifier>(r#""UPPER""#).is_err());
    assert!(serde_json::from_str::<CanonicalDecimal>(r#""1.0""#).is_err());
    assert!(serde_json::from_str::<PositiveInteger>(r#""01""#).is_err());
    assert!(serde_json::from_str::<PolicyId>(r#""bad/path""#).is_err());
}

#[test]
fn decimals_have_one_exact_fixed_point_spelling() {
    for valid in ["0", "1", "-1", "0.5", "-0.5", "10.25", "0.0001"] {
        assert_eq!(CanonicalDecimal::new(valid).unwrap().as_str(), valid);
    }

    for invalid in [
        "", "+1", "-0", "00", "01", ".5", "1.", "1.0", "1.20", "1e3", "NaN", "inf", "--1", "1..2",
    ] {
        assert!(
            CanonicalDecimal::new(invalid).is_err(),
            "accepted {invalid:?}"
        );
    }
}

#[test]
fn positive_integers_are_canonical_u64_text() {
    for valid in ["1", "20", "18446744073709551615"] {
        assert_eq!(PositiveInteger::new(valid).unwrap().as_str(), valid);
    }
    for invalid in [
        "",
        "0",
        "00",
        "01",
        "-1",
        "+1",
        "1.0",
        "1e2",
        "18446744073709551616",
        "999999999999999999999999999999999999",
    ] {
        assert!(
            PositiveInteger::new(invalid).is_err(),
            "accepted {invalid:?}"
        );
    }
}

#[test]
fn every_ast_variant_has_the_exact_documented_bytes() {
    let registry = register(&[("rolling.mean", 1, OperatorPolicy::ORDERED)]).unwrap();
    let vectors = [
        (
            field("market.close"),
            r#"{"node":"field","field":"market.close"}"#,
        ),
        (decimal("20"), r#"{"node":"decimal","value":"20"}"#),
        (
            FactorExpr::Literal(Literal::Boolean(true)),
            r#"{"node":"boolean","value":true}"#,
        ),
        (
            FactorExpr::Literal(Literal::Enumeration(EnumLiteral::new(
                identifier("rank.method"),
                identifier("average"),
            ))),
            r#"{"node":"enum","enum_type":"rank.method","value":"average"}"#,
        ),
        (
            call(
                "rolling.mean",
                1,
                vec![field("market.close"), decimal("20")],
            ),
            r#"{"node":"call","operator":"rolling.mean","operator_version":"1","arguments":[{"node":"field","field":"market.close"},{"node":"decimal","value":"20"}]}"#,
        ),
    ];

    for (expression, expected) in vectors {
        let actual =
            canonical_expression_bytes(&expression, &registry, ValidationLimits::default())
                .unwrap();
        assert_eq!(actual, expected.as_bytes());
    }
}

#[test]
fn canonical_expression_hash_has_a_fixed_cross_language_vector() {
    let expression = call(
        "rolling.mean",
        1,
        vec![field("market.close"), decimal("20")],
    );
    let registry = register(&[("rolling.mean", 1, OperatorPolicy::ORDERED)]).unwrap();
    assert_eq!(
        expression_id(&expression, &registry, ValidationLimits::default())
            .unwrap()
            .to_external(),
        "sha256:6140234b602a8fa1108888ce9fad225c4d4741f22cc261aa93b64e63d641593d"
    );
}

#[test]
fn canonical_factor_spec_matches_the_closed_field_order() {
    let expression = call(
        "rolling.mean",
        1,
        vec![field("market.close"), decimal("20")],
    );
    let registry = register(&[("rolling.mean", 1, OperatorPolicy::ORDERED)]).unwrap();
    let spec = bind_test_spec(&expression, &registry, FactorDirection::HigherIsBetter);
    let actual = String::from_utf8(canonical_factor_spec_bytes(&spec)).unwrap();
    assert!(actual.starts_with(r#"{"schema":"loop.factor-spec/v1","expression_id":"sha256:"#));
    assert!(actual.contains(r#""direction":"higher_is_better","universe_policy":"#));
    let expression_bytes =
        canonical_expression_bytes(&expression, &registry, ValidationLimits::default()).unwrap();
    let parsed = parse_canonical_factor_spec(
        actual.as_bytes(),
        factor_spec_id(&spec),
        &expression_bytes,
        &registry,
        ValidationLimits::default(),
    )
    .unwrap();
    assert_eq!(parsed, spec);
}

#[test]
fn declared_commutativity_converges_all_argument_permutations() {
    let registry = register(&[("arithmetic.add", 1, OperatorPolicy::COMMUTATIVE)]).unwrap();
    let permutations = [
        ["market.close", "market.open", "market.volume"],
        ["market.close", "market.volume", "market.open"],
        ["market.open", "market.close", "market.volume"],
        ["market.open", "market.volume", "market.close"],
        ["market.volume", "market.close", "market.open"],
        ["market.volume", "market.open", "market.close"],
    ];
    let expected = expression_id(
        &call("arithmetic.add", 1, permutations[0].map(field).to_vec()),
        &registry,
        ValidationLimits::default(),
    )
    .unwrap();

    for permutation in permutations {
        let candidate = call("arithmetic.add", 1, permutation.map(field).to_vec());
        assert_eq!(
            expression_id(&candidate, &registry, ValidationLimits::default()).unwrap(),
            expected
        );
    }
}

#[test]
fn ordered_missing_value_sensitive_operator_preserves_argument_order() {
    let registry = register(&[("arithmetic.subtract", 1, OperatorPolicy::ORDERED)]).unwrap();
    let left = call(
        "arithmetic.subtract",
        1,
        vec![field("market.close"), field("market.open")],
    );
    let right = call(
        "arithmetic.subtract",
        1,
        vec![field("market.open"), field("market.close")],
    );
    assert_ne!(
        expression_id(&left, &registry, ValidationLimits::default()).unwrap(),
        expression_id(&right, &registry, ValidationLimits::default()).unwrap()
    );
}

#[test]
fn associativity_is_scoped_to_the_exact_operator_version() {
    let registry = register(&[
        ("arithmetic.add", 1, OperatorPolicy::COMMUTATIVE_ASSOCIATIVE),
        ("arithmetic.add", 2, OperatorPolicy::ORDERED),
    ])
    .unwrap();
    let nested_v1 = call(
        "arithmetic.add",
        1,
        vec![
            field("market.close"),
            call(
                "arithmetic.add",
                1,
                vec![field("market.open"), field("market.volume")],
            ),
        ],
    );
    let flat_v1 = call(
        "arithmetic.add",
        1,
        vec![
            field("market.volume"),
            field("market.close"),
            field("market.open"),
        ],
    );
    assert_eq!(
        expression_id(&nested_v1, &registry, ValidationLimits::default()).unwrap(),
        expression_id(&flat_v1, &registry, ValidationLimits::default()).unwrap()
    );

    let nested_v2 = call(
        "arithmetic.add",
        2,
        vec![
            field("market.close"),
            call(
                "arithmetic.add",
                2,
                vec![field("market.open"), field("market.volume")],
            ),
        ],
    );
    let flat_v2 = call(
        "arithmetic.add",
        2,
        vec![
            field("market.close"),
            field("market.open"),
            field("market.volume"),
        ],
    );
    assert_ne!(
        expression_id(&nested_v2, &registry, ValidationLimits::default()).unwrap(),
        expression_id(&flat_v2, &registry, ValidationLimits::default()).unwrap()
    );
}

#[test]
fn unknown_operator_or_version_fails_closed() {
    let expression = call("rolling.mean", 2, vec![field("market.close")]);
    let registry = register(&[("rolling.mean", 1, OperatorPolicy::ORDERED)]).unwrap();
    assert!(matches!(
        expression_id(&expression, &registry, ValidationLimits::default()),
        Err(CanonicalizationError::Validation(
            ValidationError::UnknownOperator { .. }
        ))
    ));
}

#[test]
fn direction_and_each_policy_component_are_identity_material() {
    let expression = field("market.close");
    let registry = register(&[]).unwrap();
    let canonical =
        canonical_expression_bytes(&expression, &registry, ValidationLimits::default()).unwrap();
    let base_expression_id =
        expression_id(&expression, &registry, ValidationLimits::default()).unwrap();
    let base_draft = factor_spec_draft(
        base_expression_id,
        &registry,
        FactorDirection::HigherIsBetter,
    );
    let base = bind_test_draft(base_draft.clone(), &canonical, &registry);
    let changed_direction = bind_test_draft(
        factor_spec_draft(
            base_expression_id,
            &registry,
            FactorDirection::LowerIsBetter,
        ),
        &canonical,
        &registry,
    );
    let mut changed_revision_draft = base_draft.clone();
    changed_revision_draft.cost_policy = policy("us_equities_cost_v1", 2, 0x81);
    let changed_revision = bind_test_draft(changed_revision_draft, &canonical, &registry);
    let mut changed_digest_draft = base_draft.clone();
    changed_digest_draft.cost_policy = policy("us_equities_cost_v1", 1, 0x82);
    let changed_digest = bind_test_draft(changed_digest_draft, &canonical, &registry);

    let changed_expression_tree = field("market.open");
    let changed_expression_bytes = canonical_expression_bytes(
        &changed_expression_tree,
        &registry,
        ValidationLimits::default(),
    )
    .unwrap();
    let changed_expression_id = expression_id(
        &changed_expression_tree,
        &registry,
        ValidationLimits::default(),
    )
    .unwrap();
    let changed_expression = bind_test_draft(
        factor_spec_draft(
            changed_expression_id,
            &registry,
            FactorDirection::HigherIsBetter,
        ),
        &changed_expression_bytes,
        &registry,
    );

    let identities = [
        factor_spec_id(&base),
        factor_spec_id(&changed_direction),
        factor_spec_id(&changed_revision),
        factor_spec_id(&changed_digest),
        factor_spec_id(&changed_expression),
    ];
    for left in 0..identities.len() {
        for right in (left + 1)..identities.len() {
            assert_ne!(identities[left], identities[right]);
        }
    }
}

#[test]
fn depth_node_argument_and_byte_limits_fail_closed() {
    let registry = register(&[
        ("math.abs", 1, OperatorPolicy::ORDERED),
        ("arithmetic.add", 1, OperatorPolicy::ORDERED),
    ])
    .unwrap();
    let nested = call(
        "math.abs",
        1,
        vec![call("math.abs", 1, vec![field("market.close")])],
    );
    assert!(matches!(
        expression_id(
            &nested,
            &registry,
            ValidationLimits::new(2, 10, 10, 1_024).unwrap()
        ),
        Err(CanonicalizationError::Validation(
            ValidationError::DepthLimit { .. }
        ))
    ));
    assert!(matches!(
        expression_id(
            &nested,
            &registry,
            ValidationLimits::new(10, 2, 10, 1_024).unwrap()
        ),
        Err(CanonicalizationError::Validation(
            ValidationError::NodeLimit { .. }
        ))
    ));

    let wide = call(
        "arithmetic.add",
        1,
        vec![field("market.close"), field("market.open")],
    );
    assert!(matches!(
        expression_id(
            &wide,
            &registry,
            ValidationLimits::new(10, 10, 1, 1_024).unwrap()
        ),
        Err(CanonicalizationError::Validation(
            ValidationError::ArgumentLimit { .. }
        ))
    ));
    assert!(matches!(
        expression_id(
            &field("market.close"),
            &registry,
            ValidationLimits::new(10, 10, 10, 20).unwrap()
        ),
        Err(CanonicalizationError::Validation(
            ValidationError::CanonicalByteLimit { .. }
        ))
    ));
}

#[test]
fn normalized_tree_is_revalidated_after_associative_flattening() {
    let registry =
        register(&[("arithmetic.add", 1, OperatorPolicy::COMMUTATIVE_ASSOCIATIVE)]).unwrap();
    let expression = call(
        "arithmetic.add",
        1,
        vec![
            field("market.close"),
            call(
                "arithmetic.add",
                1,
                vec![field("market.open"), field("market.volume")],
            ),
        ],
    );
    assert!(matches!(
        expression_id(
            &expression,
            &registry,
            ValidationLimits::new(10, 10, 2, 1_024).unwrap()
        ),
        Err(CanonicalizationError::Validation(
            ValidationError::ArgumentLimit { .. }
        ))
    ));
}

#[test]
fn deployments_can_lower_but_not_raise_v1_safety_limits() {
    assert!(ValidationLimits::new(0, 1, 1, 1).is_err());
    assert!(ValidationLimits::new(65, 1, 1, 1).is_err());
    assert!(ValidationLimits::new(1, 4_097, 1, 1).is_err());
    assert!(ValidationLimits::new(1, 1, 1_025, 1).is_err());
    assert!(ValidationLimits::new(1, 1, 1, 262_145).is_err());
}

#[test]
fn supplied_ids_require_prefix_lowercase_hex_and_exact_recomputation() {
    let expression = field("market.close");
    let registry = register(&[]).unwrap();
    let correct = expression_id(&expression, &registry, ValidationLimits::default()).unwrap();
    ExpressionId::parse(&correct.to_external())
        .unwrap()
        .verify(&expression, &registry, ValidationLimits::default())
        .unwrap();

    assert!(ExpressionId::parse(&"A".repeat(64)).is_err());
    assert!(ExpressionId::parse(&format!("sha256:{}", "A".repeat(64))).is_err());
    assert!(ExpressionId::parse("sha256:00").is_err());
    assert!(matches!(
        ExpressionId::from_bytes([0; 32]).verify(
            &expression,
            &registry,
            ValidationLimits::default()
        ),
        Err(IdentityVerificationError::ExpressionMismatch { .. })
    ));

    let expression_bytes =
        canonical_expression_bytes(&expression, &registry, ValidationLimits::default()).unwrap();
    let spec = bind_test_spec(&expression, &registry, FactorDirection::HigherIsBetter);
    let spec_bytes = canonical_factor_spec_bytes(&spec);
    let correct_spec_id = factor_spec_id(&spec);
    FactorSpecId::parse(&correct_spec_id.to_external())
        .unwrap()
        .verify(
            &spec_bytes,
            &expression_bytes,
            &registry,
            ValidationLimits::default(),
        )
        .unwrap();
    assert!(matches!(
        FactorSpecId::from_bytes([0; 32]).verify(
            &spec_bytes,
            &expression_bytes,
            &registry,
            ValidationLimits::default(),
        ),
        Err(IdentityVerificationError::FactorSpecMismatch { .. })
    ));
}

#[test]
fn a_registry_entry_cannot_be_silently_redefined() {
    let (mut registry, _) =
        register_builder(&[("arithmetic.add", 1, OperatorPolicy::COMMUTATIVE)]).unwrap();
    let contract = semantic_contract("arithmetic.add", 1);
    let duplicate = OperatorDefinition::variadic(
        operator("arithmetic.add", 1),
        vec![],
        ArgumentDefinition::series(),
        2,
        ValidationLimits::HARD_MAX_ARGUMENTS,
        ValueType::Series,
        OperatorPolicy::ORDERED,
        contract.identity(),
    )
    .unwrap();
    assert!(matches!(
        registry.register_operator(duplicate),
        Err(RegistryError::DuplicateOperator { .. })
    ));
}

fn conformance_fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/contracts/factor/v1/canonical_vectors.json"
    )))
    .expect("shared canonical fixture must be valid JSON")
}

fn fixture_value_type(raw: &serde_json::Value) -> ValueType {
    if let Some(value) = raw.as_str() {
        return match value {
            "series" => ValueType::Series,
            "decimal" => ValueType::Decimal,
            "boolean" => ValueType::Boolean,
            other => panic!("unknown fixture value type: {other}"),
        };
    }
    ValueType::Enumeration(identifier(raw["enumType"].as_str().unwrap()))
}

fn fixture_argument(raw: &serde_json::Value) -> ArgumentDefinition {
    let decimal = raw.get("decimal").map(|constraints| {
        DecimalConstraints::new(
            constraints["maxPrecision"].as_u64().unwrap() as usize,
            constraints["maxScale"].as_u64().unwrap() as usize,
            CanonicalDecimal::new(constraints["minimum"].as_str().unwrap()).unwrap(),
            CanonicalDecimal::new(constraints["maximum"].as_str().unwrap()).unwrap(),
        )
        .unwrap()
    });
    ArgumentDefinition::new(
        fixture_value_type(&raw["type"]),
        raw.get("literalOnly")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        decimal,
    )
    .unwrap()
}

fn fixture_registry(raw: &serde_json::Value) -> OperatorPolicyRegistry {
    let mut registry = OperatorRegistryBuilder::new();
    let mut contracts = BTreeMap::new();
    let semantic_fixture = semantic_conformance_fixture();
    for field in raw["fields"].as_array().unwrap() {
        registry
            .register_field(
                identifier(field["field"].as_str().unwrap()),
                fixture_value_type(&field["outputType"]),
            )
            .unwrap();
    }
    for enumeration in raw["enums"].as_array().unwrap() {
        registry
            .register_enum(
                identifier(enumeration["enumType"].as_str().unwrap()),
                enumeration["values"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| identifier(value.as_str().unwrap())),
            )
            .unwrap();
    }
    for raw_definition in raw["operators"].as_array().unwrap() {
        let operator = OperatorRef::new(
            identifier(raw_definition["operator"].as_str().unwrap()),
            PositiveInteger::new(raw_definition["operatorVersion"].as_str().unwrap()).unwrap(),
        );
        let semantic_contract_sha256 =
            SemanticContractId::parse(raw_definition["semanticContractSha256"].as_str().unwrap())
                .unwrap();
        let semantic_raw = semantic_fixture["accepted"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["sha256"].as_str().unwrap() == semantic_contract_sha256.to_string())
            .expect("registry semantic contract must be present in shared fixture");
        contracts.insert(
            semantic_contract_sha256,
            semantic_raw["canonical_utf8"]
                .as_str()
                .unwrap()
                .as_bytes()
                .to_vec(),
        );
        let parameters = raw_definition["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .map(fixture_argument)
            .collect();
        let output = fixture_value_type(&raw_definition["outputType"]);
        let policy = OperatorPolicy::new(
            raw_definition["commutative"].as_bool().unwrap(),
            raw_definition["associative"].as_bool().unwrap(),
        );
        let definition = if let Some(variadic) = raw_definition.get("variadic") {
            OperatorDefinition::variadic(
                operator,
                parameters,
                fixture_argument(variadic),
                raw_definition["minArguments"].as_u64().unwrap() as usize,
                raw_definition["maxArguments"].as_u64().unwrap() as usize,
                output,
                policy,
                semantic_contract_sha256,
            )
        } else {
            OperatorDefinition::fixed(
                operator,
                parameters,
                output,
                policy,
                semantic_contract_sha256,
            )
        }
        .unwrap();
        registry.register_operator(definition).unwrap();
    }
    registry.build(&contracts).unwrap()
}

fn fixture_operator_definition(raw: &serde_json::Value) -> Result<OperatorDefinition, String> {
    let operator_ref = OperatorRef::new(
        Identifier::new(
            raw["operator"]
                .as_str()
                .ok_or("operator must be a string")?,
        )
        .map_err(|error| error.to_string())?,
        PositiveInteger::new(
            raw["operatorVersion"]
                .as_str()
                .ok_or("operatorVersion must be a string")?,
        )
        .map_err(|error| error.to_string())?,
    );
    let parameters = raw["parameters"]
        .as_array()
        .ok_or("parameters must be an array")?
        .iter()
        .map(fixture_argument)
        .collect();
    let output = fixture_value_type(&raw["outputType"]);
    let policy = OperatorPolicy::new(
        raw["commutative"]
            .as_bool()
            .ok_or("commutative must be a boolean")?,
        raw["associative"]
            .as_bool()
            .ok_or("associative must be a boolean")?,
    );
    let variadic = raw
        .get("variadic")
        .ok_or("fixture definition must be variadic")?;
    let semantic_contract_sha256 = semantic_contract(
        operator_ref.name().as_str(),
        operator_ref.semantic_version().as_str().parse().unwrap(),
    )
    .identity();
    OperatorDefinition::variadic(
        operator_ref,
        parameters,
        fixture_argument(variadic),
        raw["minArguments"]
            .as_u64()
            .ok_or("minArguments must be a non-negative integer")? as usize,
        raw["maxArguments"]
            .as_u64()
            .ok_or("maxArguments must be a non-negative integer")? as usize,
        output,
        policy,
        semantic_contract_sha256,
    )
    .map_err(|error| error.to_string())
}

fn semantic_conformance_fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/contracts/factor/v1/operator_semantic_contract_vectors.json"
    )))
    .expect("shared semantic contract fixture must be valid JSON")
}

#[test]
fn shared_semantic_contract_vectors_are_exact_and_fail_closed() {
    let fixture = semantic_conformance_fixture();
    for vector in fixture["accepted"].as_array().unwrap() {
        let bytes = vector["canonical_utf8"].as_str().unwrap().as_bytes();
        let contract = parse_canonical_operator_semantic_contract(bytes).unwrap();
        assert_eq!(
            semantic_contract_sha256(bytes).to_string(),
            vector["sha256"].as_str().unwrap(),
            "{}",
            vector["name"].as_str().unwrap()
        );
        assert!(!contract.operator().as_str().is_empty());
    }
    for (policy, variants) in fixture["policy_variants"].as_object().unwrap() {
        for variant in variants.as_array().unwrap() {
            let mut null_policy = "not_applicable";
            let mut window_policy = "not_applicable";
            let mut tie_policy = "not_applicable";
            let mut alignment_policy = "not_applicable";
            let mut numeric_policy = "not_applicable";
            match policy.as_str() {
                "nullPolicy" => null_policy = variant.as_str().unwrap(),
                "windowPolicy" => window_policy = variant.as_str().unwrap(),
                "tiePolicy" => tie_policy = variant.as_str().unwrap(),
                "alignmentPolicy" => alignment_policy = variant.as_str().unwrap(),
                "numericPolicy" => numeric_policy = variant.as_str().unwrap(),
                unknown => panic!("unknown semantic policy fixture: {unknown}"),
            }
            let canonical = format!(
                "{{\"schema\":\"loop.operator-semantic-contract/v1\",\"operator\":\"fixture.semantic\",\"operatorVersion\":\"1\",\"nullPolicy\":\"{null_policy}\",\"windowPolicy\":\"{window_policy}\",\"tiePolicy\":\"{tie_policy}\",\"alignmentPolicy\":\"{alignment_policy}\",\"numericPolicy\":\"{numeric_policy}\"}}"
            );
            assert!(
                parse_canonical_operator_semantic_contract(canonical.as_bytes()).is_ok(),
                "rejected {policy}={variant}"
            );
        }
    }
    for rejected in fixture["rejected_canonical_utf8"].as_array().unwrap() {
        assert!(
            parse_canonical_operator_semantic_contract(rejected.as_str().unwrap().as_bytes())
                .is_err()
        );
    }
    let depth = fixture["deep_nesting"].as_u64().unwrap() as usize;
    let deep = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
    assert!(parse_canonical_operator_semantic_contract(deep.as_bytes()).is_err());
}

#[test]
fn registry_fails_closed_on_unresolved_misaddressed_and_misbound_semantics() {
    let (builder, contracts) =
        register_builder(&[("arithmetic.add", 1, OperatorPolicy::COMMUTATIVE_ASSOCIATIVE)])
            .unwrap();
    assert!(matches!(
        builder.clone().build(&BTreeMap::new()),
        Err(RegistryError::SemanticContractNotFound { .. })
    ));

    let (identity, bytes) = contracts.into_iter().next().unwrap();
    let mut corrupt = BTreeMap::new();
    let mut changed_bytes = bytes;
    changed_bytes.push(b' ');
    corrupt.insert(identity, changed_bytes);
    assert!(matches!(
        builder.build(&corrupt),
        Err(RegistryError::SemanticContractDigestMismatch { .. })
    ));

    let contract = semantic_contract("arithmetic.subtract", 1);
    let mut misbound_contracts = BTreeMap::new();
    misbound_contracts.insert(contract.identity(), contract.canonical_bytes());
    let mut misbound_builder = OperatorRegistryBuilder::new();
    misbound_builder
        .register_operator(
            OperatorDefinition::variadic(
                operator("arithmetic.add", 1),
                vec![],
                ArgumentDefinition::series(),
                2,
                ValidationLimits::HARD_MAX_ARGUMENTS,
                ValueType::Series,
                OperatorPolicy::COMMUTATIVE_ASSOCIATIVE,
                contract.identity(),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(matches!(
        misbound_builder.build(&misbound_contracts),
        Err(RegistryError::SemanticContractOperatorMismatch { .. })
    ));
}

fn fixture_node(raw: &serde_json::Value) -> Result<FactorExpr, String> {
    match raw["node"].as_str().ok_or("node must be a string")? {
        "field" => Ok(FactorExpr::Field(FieldRef::new(
            Identifier::new(raw["field"].as_str().ok_or("field must be a string")?)
                .map_err(|error| error.to_string())?,
        ))),
        "decimal" => Ok(FactorExpr::Literal(Literal::Decimal(
            CanonicalDecimal::new(raw["value"].as_str().ok_or("value must be a string")?)
                .map_err(|error| error.to_string())?,
        ))),
        "boolean" => Ok(FactorExpr::Literal(Literal::Boolean(
            raw["value"].as_bool().ok_or("value must be a boolean")?,
        ))),
        "enum" => Ok(FactorExpr::Literal(Literal::Enumeration(EnumLiteral::new(
            Identifier::new(
                raw["enum_type"]
                    .as_str()
                    .ok_or("enum_type must be a string")?,
            )
            .map_err(|error| error.to_string())?,
            Identifier::new(raw["value"].as_str().ok_or("value must be a string")?)
                .map_err(|error| error.to_string())?,
        )))),
        "call" => Ok(FactorExpr::Call(OperatorCall::new(
            OperatorRef::new(
                Identifier::new(
                    raw["operator"]
                        .as_str()
                        .ok_or("operator must be a string")?,
                )
                .map_err(|error| error.to_string())?,
                PositiveInteger::new(
                    raw["operator_version"]
                        .as_str()
                        .ok_or("operator_version must be a string")?,
                )
                .map_err(|error| error.to_string())?,
            ),
            raw["arguments"]
                .as_array()
                .ok_or("arguments must be an array")?
                .iter()
                .map(fixture_node)
                .collect::<Result<Vec<_>, _>>()?,
        ))),
        node => Err(format!("unknown fixture node: {node}")),
    }
}

fn fixture_digest(value: &str) -> [u8; 32] {
    *ExpressionId::parse(value).unwrap().as_bytes()
}

fn fixture_policy(raw: &serde_json::Value) -> PolicyRef {
    PolicyRef::new(
        PolicyId::new(raw["policy_id"].as_str().unwrap()).unwrap(),
        PositiveInteger::new(raw["revision"].as_str().unwrap()).unwrap(),
        fixture_digest(raw["sha256"].as_str().unwrap()),
    )
}

fn fixture_factor_spec_draft(raw: &serde_json::Value) -> FactorSpecDraft {
    FactorSpecDraft::new(
        ExpressionId::parse(raw["expression_id"].as_str().unwrap()).unwrap(),
        fixture_digest(raw["operator_registry_sha256"].as_str().unwrap()),
        match raw["direction"].as_str().unwrap() {
            "higher_is_better" => FactorDirection::HigherIsBetter,
            "lower_is_better" => FactorDirection::LowerIsBetter,
            direction => panic!("invalid fixture direction: {direction}"),
        },
        fixture_policy(&raw["universe_policy"]),
        fixture_policy(&raw["data_policy"]),
        fixture_policy(&raw["calendar_policy"]),
        fixture_policy(&raw["preprocess_policy"]),
        fixture_policy(&raw["neutralization_policy"]),
        fixture_policy(&raw["portfolio_policy"]),
        fixture_policy(&raw["execution_policy"]),
        fixture_policy(&raw["cost_policy"]),
        fixture_policy(&raw["evaluation_policy"]),
    )
}

#[test]
fn shared_expression_vectors_are_byte_and_id_exact() {
    let fixture = conformance_fixture();
    let registry = fixture_registry(&fixture["registry"]);
    for vector in fixture["expression_vectors"].as_array().unwrap() {
        let expression = fixture_node(&vector["input"]).unwrap();
        let canonical =
            canonical_expression_bytes(&expression, &registry, ValidationLimits::default())
                .unwrap();
        assert_eq!(
            canonical,
            vector["canonical_utf8"].as_str().unwrap().as_bytes(),
            "{}",
            vector["name"]
        );
        assert_eq!(
            expression_id(&expression, &registry, ValidationLimits::default())
                .unwrap()
                .to_external(),
            vector["expression_id"].as_str().unwrap(),
            "{}",
            vector["name"]
        );
        assert_eq!(
            parse_canonical_expression(&canonical, &registry, ValidationLimits::default()).unwrap(),
            fixture_node(&serde_json::from_slice(&canonical).unwrap()).unwrap()
        );
    }
}

#[test]
fn shared_registry_boundary_and_scalar_type_vectors() {
    let fixture = conformance_fixture();
    let registry = fixture_registry(&fixture["registry"]);
    assert_eq!(
        registry.canonical_bytes(),
        fixture["registry_canonical_utf8"]
            .as_str()
            .unwrap()
            .as_bytes()
    );
    assert_eq!(
        registry.identity().to_external(),
        fixture["registry_sha256"].as_str().unwrap()
    );
    let rolling = operator("rolling.mean", 1);
    assert_eq!(
        registry
            .semantic_contract_for(&rolling)
            .unwrap()
            .null_policy(),
        NullPolicy::IgnoreMissing
    );
    let vectors = &fixture["registry_operator_vectors"];
    for vector in vectors["accepted"].as_array().unwrap() {
        assert!(
            fixture_operator_definition(&vector["definition"]).is_ok(),
            "rejected {}",
            vector["name"]
        );
    }
    for vector in vectors["rejected"].as_array().unwrap() {
        assert!(
            fixture_operator_definition(&vector["definition"]).is_err(),
            "accepted {}",
            vector["name"]
        );
    }
}

#[test]
fn shared_factor_spec_vector_binds_registry_and_all_policies() {
    let fixture = conformance_fixture();
    let registry = fixture_registry(&fixture["registry"]);
    let vector = &fixture["factor_spec_vectors"][0];
    let expression_bytes = vector["expression_canonical_utf8"]
        .as_str()
        .unwrap()
        .as_bytes();
    let draft = fixture_factor_spec_draft(&vector["input"]);
    let spec = bind_test_draft(draft.clone(), expression_bytes, &registry);
    assert_eq!(
        canonical_factor_spec_bytes(&spec),
        vector["canonical_utf8"].as_str().unwrap().as_bytes()
    );
    assert_eq!(
        factor_spec_id(&spec).to_external(),
        vector["factor_spec_id"].as_str().unwrap()
    );
    let parsed = parse_canonical_factor_spec(
        vector["canonical_utf8"].as_str().unwrap().as_bytes(),
        FactorSpecId::parse(vector["factor_spec_id"].as_str().unwrap()).unwrap(),
        expression_bytes,
        &registry,
        ValidationLimits::default(),
    )
    .unwrap();
    assert_eq!(parsed, spec);
    for vector in fixture["rejected_registry_bindings"].as_array().unwrap() {
        let mut changed_draft = draft.clone();
        changed_draft.operator_registry_sha256 =
            fixture_digest(vector["operator_registry_sha256"].as_str().unwrap());
        assert!(
            matches!(
                bind_factor_spec(
                    changed_draft,
                    expression_bytes,
                    &registry,
                    ValidationLimits::default(),
                ),
                Err(IdentityVerificationError::Canonicalization(
                    CanonicalizationError::OperatorRegistryMismatch { .. }
                ))
            ),
            "accepted {}",
            vector["name"]
        );
    }
}

#[test]
fn shared_non_series_roots_cannot_bind_factor_specs() {
    let fixture = conformance_fixture();
    let registry = fixture_registry(&fixture["registry"]);
    let template = fixture_factor_spec_draft(&fixture["factor_spec_vectors"][0]["input"]);
    for vector in fixture["rejected_factor_bindings"].as_array().unwrap() {
        let mut draft = template.clone();
        draft.expression_id =
            ExpressionId::parse(vector["expression_id"].as_str().unwrap()).unwrap();
        let result = bind_factor_spec(
            draft,
            vector["canonical_expression_utf8"]
                .as_str()
                .unwrap()
                .as_bytes(),
            &registry,
            ValidationLimits::default(),
        );
        assert!(
            matches!(
                result,
                Err(IdentityVerificationError::Canonicalization(
                    CanonicalizationError::NonSeriesFactorRoot { .. }
                ))
            ),
            "accepted {}",
            vector["name"]
        );
    }
}

#[test]
fn canonical_factor_spec_parser_rejects_every_shared_malformed_form() {
    let fixture = conformance_fixture();
    let registry = fixture_registry(&fixture["registry"]);
    let vector = &fixture["factor_spec_vectors"][0];
    let canonical = vector["canonical_utf8"].as_str().unwrap();
    let expression_bytes = vector["expression_canonical_utf8"]
        .as_str()
        .unwrap()
        .as_bytes();
    let expected_id = FactorSpecId::parse(vector["factor_spec_id"].as_str().unwrap()).unwrap();
    for mutation in fixture["rejected_factor_spec_canonical_mutations"]
        .as_array()
        .unwrap()
    {
        let mutated = mutate_factor_spec(canonical, mutation.as_str().unwrap());
        assert!(
            parse_canonical_factor_spec(
                mutated.as_bytes(),
                expected_id,
                expression_bytes,
                &registry,
                ValidationLimits::default(),
            )
            .is_err(),
            "accepted {}",
            mutation
        );
    }
}

fn mutate_factor_spec(canonical: &str, mutation: &str) -> String {
    match mutation {
        "leading_whitespace" => format!(" {canonical}"),
        "reordered_top_level_fields" => canonical.replacen(
            r#"{"schema":"loop.factor-spec/v1","expression_id":"#,
            r#"{"expression_id":"#,
            1,
        ).replacen(
            r#"","operator_registry_sha256":"#,
            r#"","schema":"loop.factor-spec/v1","operator_registry_sha256":"#,
            1,
        ),
        "duplicate_direction" => canonical.replacen(
            r#""direction":"higher_is_better","#,
            r#""direction":"higher_is_better","direction":"higher_is_better","#,
            1,
        ),
        "unknown_top_level_field" => format!("{},\"unknown\":true}}", &canonical[..canonical.len() - 1]),
        "missing_evaluation_policy" => {
            let start = canonical.find(r#","evaluation_policy":"#).unwrap();
            format!("{}}}", &canonical[..start])
        }
        "reordered_policy_fields" => canonical.replacen(
            r#""universe_policy":{"policy_id":"us_common_stock","revision":"1","#,
            r#""universe_policy":{"revision":"1","policy_id":"us_common_stock","#,
            1,
        ),
        "duplicate_policy_field" => canonical.replacen(
            r#""universe_policy":{"policy_id":"us_common_stock","#,
            r#""universe_policy":{"policy_id":"us_common_stock","policy_id":"us_common_stock","#,
            1,
        ),
        "unknown_policy_field" => canonical.replacen(
            "\"sha256\":\"sha256:0000000000000000000000000000000000000000000000000000000000000001\"}",
            "\"sha256\":\"sha256:0000000000000000000000000000000000000000000000000000000000000001\",\"unknown\":true}",
            1,
        ),
        "missing_policy_field" => canonical.replacen(
            ",\"sha256\":\"sha256:0000000000000000000000000000000000000000000000000000000000000001\"",
            "",
            1,
        ),
        "wrong_schema" => canonical.replacen("loop.factor-spec/v1", "loop.factor-spec/v2", 1),
        "auto_direction" => canonical.replacen("higher_is_better", "auto", 1),
        other => panic!("unknown FactorSpec mutation: {other}"),
    }
}

#[test]
fn shared_scalar_and_semantic_negative_vectors_fail_closed() {
    let fixture = conformance_fixture();
    let registry = fixture_registry(&fixture["registry"]);
    for value in fixture["accepted_decimals"].as_array().unwrap() {
        let expression = fixture_node(&serde_json::json!({
            "node": "decimal",
            "value": value,
        }))
        .unwrap();
        canonical_expression_bytes(&expression, &registry, ValidationLimits::default()).unwrap();
    }
    for value in fixture["accepted_positive_integers"].as_array().unwrap() {
        assert_eq!(
            PositiveInteger::new(value.as_str().unwrap())
                .unwrap()
                .as_str(),
            value.as_str().unwrap()
        );
    }
    for value in fixture["rejected_decimals"].as_array().unwrap() {
        assert!(fixture_node(&serde_json::json!({ "node": "decimal", "value": value })).is_err());
    }
    for value in fixture["rejected_positive_integers"].as_array().unwrap() {
        assert!(PositiveInteger::new(value.as_str().unwrap()).is_err());
    }
    for value in fixture["rejected_identifiers"].as_array().unwrap() {
        assert!(fixture_node(&serde_json::json!({ "node": "field", "field": value })).is_err());
    }
    for vector in fixture["rejected_expressions"].as_array().unwrap() {
        let limits = vector
            .get("limit_overrides")
            .and_then(|overrides| overrides.get("max_direct_arguments"))
            .and_then(serde_json::Value::as_u64)
            .map_or_else(ValidationLimits::default, |maximum| {
                ValidationLimits::new(64, 4_096, maximum as usize, 262_144).unwrap()
            });
        let result = fixture_node(&vector["input"]).and_then(|expression| {
            canonical_expression_bytes(&expression, &registry, limits)
                .map(|_| ())
                .map_err(|error| error.to_string())
        });
        assert!(result.is_err(), "accepted {}", vector["name"]);
    }
    for canonical in fixture["rejected_canonical_utf8"].as_array().unwrap() {
        assert!(
            parse_canonical_expression(
                canonical.as_str().unwrap().as_bytes(),
                &registry,
                ValidationLimits::default(),
            )
            .is_err(),
            "accepted alternate canonical spelling: {canonical}"
        );
    }
    for direction in fixture["rejected_directions"].as_array().unwrap() {
        assert!(serde_json::from_value::<FactorDirection>(direction.clone()).is_err());
    }
}
