//! Closed execution contract for the installed US-equity evaluator, version 2.
//! This module declares semantics only; numerical implementations remain Python.

use super::*;

/// Build the exact registry accepted by the fixed numerical worker.
/// Unknown versions are not inherited. Binary64 addition and multiplication
/// commute their two arguments but are deliberately not associative.
pub fn registry() -> Result<OperatorPolicyRegistry, RegistryError> {
    let identifier = |name: &str| {
        Identifier::new(name)
            .map_err(|error| RegistryError::InvalidSemanticContract(error.to_string()))
    };
    let version = PositiveInteger::new("2")
        .map_err(|error| RegistryError::InvalidSemanticContract(error.to_string()))?;
    let decimal = |value: &str| {
        CanonicalDecimal::new(value)
            .map_err(|error| RegistryError::InvalidDecimalConstraints(error.to_string()))
    };
    let count = ArgumentDefinition::decimal_literal(DecimalConstraints::new(
        4,
        0,
        decimal("1")?,
        decimal("4096")?,
    )?);
    let mut builder = OperatorRegistryBuilder::new();
    for field in ["open", "high", "low", "close", "adjusted_close", "volume"] {
        builder.register_field(identifier(&format!("market.{field}"))?, ValueType::Series)?;
    }
    let mut contracts = BTreeMap::new();
    for name in [
        "add", "delta", "div", "ma", "max", "min", "mul", "rank_cs", "rank_ts", "roc", "skew",
        "std", "sub", "zscore",
    ] {
        let rolling = matches!(name, "ma" | "std" | "min" | "max" | "skew" | "rank_ts");
        let lagged = matches!(name, "delta" | "roc");
        let binary = matches!(name, "add" | "sub" | "mul" | "div");
        let contract = OperatorSemanticContract::new(
            identifier(name)?,
            version.clone(),
            if matches!(name, "rank_ts" | "rank_cs" | "zscore") {
                NullPolicy::PreserveTargetIgnorePeers
            } else if rolling {
                NullPolicy::IgnoreMissing
            } else {
                NullPolicy::Propagate
            },
            if rolling {
                WindowPolicy::TrailingExplicitMinimum
            } else if lagged {
                WindowPolicy::LagArgument2
            } else {
                WindowPolicy::NotApplicable
            },
            match name {
                "rank_ts" => TiePolicy::TargetLastStableOrderValidCountMinusOneConstantMidpoint,
                "rank_cs" => TiePolicy::AverageValidCount,
                _ => TiePolicy::NotApplicable,
            },
            if binary {
                AlignmentPolicy::StrictTimestampAndSecurity
            } else {
                AlignmentPolicy::UnaryPreserveTimestampAndSecurity
            },
            match name {
                "std" => NumericPolicy::SampleStd,
                "skew" => NumericPolicy::AdjustedSkew,
                "zscore" => NumericPolicy::SampleZscore,
                "rank_ts" | "rank_cs" => NumericPolicy::OrdinalUnitInterval,
                _ => NumericPolicy::Binary64NonFiniteToMissing,
            },
        );
        let series = ArgumentDefinition::series();
        let parameters = if rolling {
            vec![series, count.clone(), count.clone()]
        } else if lagged {
            vec![series, count.clone()]
        } else if binary {
            vec![series.clone(), series]
        } else {
            vec![series]
        };
        builder.register_operator(OperatorDefinition::fixed(
            OperatorRef::new(identifier(name)?, version.clone()),
            parameters,
            ValueType::Series,
            OperatorPolicy::new(matches!(name, "add" | "mul"), false),
            contract.identity(),
        )?)?;
        contracts.insert(contract.identity(), contract.canonical_bytes());
    }
    builder.build(&contracts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_matches_the_installed_python_contract() {
        assert_eq!(
            registry().unwrap().identity().to_string(),
            "sha256:1e61b2328c791e46a58bf61232307c14a7100973d4540a7061f87a6df7480c34"
        );
    }
}
