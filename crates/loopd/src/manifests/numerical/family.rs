use super::*;
use crate::manifests::feedback::FrozenFeedback;
use crate::research_worker::ResearchBuild;
use factor::{CanonicalDecimal, FactorExpr, Literal, OperatorCall, WindowPolicy};
use sha2::{Digest, Sha256};

/// A run-local family whose candidate IDs come from the installed registry.
/// Portfolio observations retain their own provenance, never this base context.
pub(crate) struct VerifiedFamily {
    digest: [u8; 32],
    recipe: ObjectRef,
    feedback: ObjectRef,
    space: wire::PerturbationSpace,
    candidates: Vec<wire::CanonicalFactorAst>,
    files: Vec<Arc<VerifiedFile>>,
}

#[derive(Serialize)]
struct FamilyDocument<'a> {
    schema: &'static str,
    recipe: &'a ObjectRef,
    context: &'a ObjectRef,
    feedback: &'a ObjectRef,
    masked_expression: &'a str,
    candidates: Vec<(u32, &'a str)>,
}

impl FrozenNumerical {
    /// Derive a single-window family or report that the first proposal has no
    /// eligible window. Invalid canonical input remains an error.
    pub(crate) fn family(
        &self,
        ast: &wire::CanonicalFactorAst,
        feedback: &FrozenFeedback,
    ) -> StoreResult<Option<VerifiedFamily>> {
        self.check()?;
        feedback.check()?;
        self.evaluation(ast, wire::JobBudget::default())?;
        let registry = &self.context.registry;
        let limits = ValidationLimits::default();
        let expression = factor::parse_canonical_expression(&ast.canonical_json, registry, limits)
            .map_err(|_| StoreError::Invalid("family canonical expression"))?;
        let path = &feedback.policy().window_path;
        let first_window = *feedback
            .policy()
            .eligible_windows
            .first()
            .ok_or(StoreError::Invalid("family windows"))?;
        let Some(masked) = replace_window(&expression, path, first_window, registry)? else {
            return Ok(None);
        };
        let masked_bytes = factor::canonical_expression_bytes(&masked, registry, limits)
            .map_err(|_| StoreError::Invalid("family masked expression"))?;
        let mut candidates = Vec::new();
        let mut windows = Vec::new();
        for window in &feedback.policy().eligible_windows {
            let Some(changed) = replace_window(&expression, path, *window, registry)? else {
                return Ok(None);
            };
            if factor::canonicalize_expression(&changed, registry, limits)
                .map_err(|_| StoreError::Invalid("family canonical window"))?
                != changed
            {
                // Reordering a commutative ancestor moves the frozen path. It
                // makes this family inapplicable, not a rejected factor.
                return Ok(None);
            }
            let mut candidate = ast.clone();
            candidate.expression_id = Some(wire::FactorExpressionId {
                value: factor::expression_id(&changed, registry, limits)
                    .map_err(|_| StoreError::Invalid("family expression ID"))?
                    .to_string(),
            });
            candidate.canonical_json =
                factor::canonical_expression_bytes(&changed, registry, limits)
                    .map_err(|_| StoreError::Invalid("family expression bytes"))?;
            wire_window(
                candidate
                    .ast
                    .as_mut()
                    .and_then(|ast| ast.root.as_mut())
                    .ok_or(StoreError::Invalid("family wire AST"))?,
                path,
                *window,
            )?;
            let input = self.evaluation(&candidate, wire::JobBudget::default())?;
            let id = input
                .factor
                .and_then(|factor| factor.factor_spec_id)
                .ok_or(StoreError::Corrupt("family factor identity"))?;
            windows.push(wire::WindowCandidate {
                window: *window,
                factor_spec_id: Some(id),
            });
            candidates.push(candidate);
        }
        let document = FamilyDocument {
            schema: "loop.run-window-family/v1",
            recipe: &self.reference,
            context: &self.recipe.evaluation_context,
            feedback: feedback.reference(),
            masked_expression: std::str::from_utf8(&masked_bytes)
                .map_err(|_| StoreError::Corrupt("family expression UTF-8"))?,
            candidates: windows
                .iter()
                .map(|candidate| {
                    Ok((
                        candidate.window,
                        candidate
                            .factor_spec_id
                            .as_ref()
                            .ok_or(StoreError::Corrupt("family candidate"))?
                            .value
                            .as_str(),
                    ))
                })
                .collect::<StoreResult<_>>()?,
        };
        let bytes =
            serde_json::to_vec(&document).map_err(|_| StoreError::Corrupt("family encoding"))?;
        let digest = Sha256::digest(bytes).into();
        let mut files = self.files.clone();
        files.extend(feedback.guards());
        Ok(Some(VerifiedFamily {
            digest,
            recipe: self.reference.clone(),
            feedback: feedback.reference().clone(),
            space: wire::PerturbationSpace {
                algorithm: "window.ema-gradient-pcg64.v1".to_owned(),
                dataset: Some(
                    self.context
                        .dataset
                        .reference(&self.context.manifest.data)?,
                ),
                provenance: Some(self.context.provenance.clone()),
                backtest_seed: Some(model::digest(model::parse_digest(
                    &self.recipe.deterministic_seed,
                )?)),
                random_seed: Some(model::digest(model::parse_digest(
                    &feedback.policy().random_seed,
                )?)),
                candidates: windows,
            },
            candidates,
            files,
        }))
    }

    pub(crate) fn build(&self) -> StoreResult<ResearchBuild> {
        self.check()?;
        Ok(ResearchBuild {
            source_sha256: self.context.manifest.source.digest()?,
            environment_sha256: self.context.manifest.environment.digest()?,
        })
    }

    pub(in crate::manifests) fn guards(&self) -> Vec<Arc<VerifiedFile>> {
        self.files.clone()
    }
}

impl VerifiedFamily {
    pub(in crate::manifests) fn binds(
        &self,
        numerical: &FrozenNumerical,
        feedback: &FrozenFeedback,
    ) -> bool {
        &self.recipe == numerical.recipe() && &self.feedback == feedback.reference()
    }
    pub(crate) fn check(&self) -> StoreResult<()> {
        check_files(&self.files)
    }
    pub(crate) fn digest(&self) -> [u8; 32] {
        self.digest
    }
    pub(crate) fn space(&self) -> &wire::PerturbationSpace {
        &self.space
    }
    pub(crate) fn candidate(&self, id: &wire::FactorSpecId) -> Option<&wire::WindowCandidate> {
        self.space
            .candidates
            .iter()
            .find(|candidate| candidate.factor_spec_id.as_ref() == Some(id))
    }
    pub(crate) fn ast(
        &self,
        candidate: &wire::WindowCandidate,
    ) -> StoreResult<&wire::CanonicalFactorAst> {
        self.check()?;
        self.space
            .candidates
            .iter()
            .position(|known| known == candidate)
            .and_then(|index| self.candidates.get(index))
            .ok_or(StoreError::Corrupt("unknown family candidate"))
    }
    pub(in crate::manifests) fn guards(&self) -> Vec<Arc<VerifiedFile>> {
        self.files.clone()
    }
}

fn replace_window(
    expression: &FactorExpr,
    path: &[u32],
    window: u32,
    registry: &factor::OperatorPolicyRegistry,
) -> StoreResult<Option<FactorExpr>> {
    let FactorExpr::Call(call) = expression else {
        return Ok(None);
    };
    let Some((&index, rest)) = path.split_first() else {
        return Ok(None);
    };
    let mut arguments = call.arguments().to_vec();
    let Some(child) = arguments.get_mut(index as usize) else {
        return Ok(None);
    };
    if rest.is_empty() {
        let semantic = registry
            .semantic_contract_for(call.operator())
            .ok_or(StoreError::Corrupt("family operator semantics"))?;
        if index != 1 || !matches!(semantic.window_policy(),
            WindowPolicy::TrailingArgument2FullWindowRightInclusiveConstantPreserve
            | WindowPolicy::TrailingArgument2MinimumValidMinNMax3Floor2NDiv3RightInclusiveConstantPreserve
            | WindowPolicy::TrailingExplicitMinimum)
            || !matches!(child, FactorExpr::Literal(Literal::Decimal(_)))
        { return Ok(None) }
        if semantic.window_policy() == WindowPolicy::TrailingExplicitMinimum
            && !matches!(call.arguments().get(2), Some(FactorExpr::Literal(Literal::Decimal(minimum)))
                if minimum.as_str().parse::<u32>().is_ok_and(|minimum| minimum <= window))
        {
            return Ok(None);
        }
        *child = FactorExpr::Literal(Literal::Decimal(
            CanonicalDecimal::new(window.to_string())
                .map_err(|_| StoreError::Invalid("family window"))?,
        ));
    } else {
        let Some(changed) = replace_window(child, rest, window, registry)? else {
            return Ok(None);
        };
        *child = changed;
    }
    Ok(Some(FactorExpr::Call(OperatorCall::new(
        call.operator().clone(),
        arguments,
    ))))
}

fn wire_window(node: &mut wire::FactorAstNode, path: &[u32], window: u32) -> StoreResult<()> {
    let Some(wire::factor_ast_node::Node::Call(call)) = &mut node.node else {
        return Err(StoreError::Corrupt("family wire parent"));
    };
    let Some((&index, rest)) = path.split_first() else {
        return Err(StoreError::Corrupt("family wire path"));
    };
    let child = call
        .arguments
        .get_mut(index as usize)
        .ok_or(StoreError::Corrupt("family wire index"))?;
    if rest.is_empty() {
        child.node = Some(wire::factor_ast_node::Node::Literal(wire::FactorLiteral {
            value: Some(wire::factor_literal::Value::Decimal(wire::ExactDecimal {
                value: window.to_string(),
            })),
        }));
        Ok(())
    } else {
        wire_window(child, rest, window)
    }
}
