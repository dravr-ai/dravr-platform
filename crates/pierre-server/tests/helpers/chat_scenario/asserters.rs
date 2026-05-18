// ABOUTME: Per-assertion dispatch from AssertionSpec into messaging_eval helpers + new asserters
// ABOUTME: Each variant of AssertionSpec maps to one function here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Assertion dispatch.
//!
//! Each variant of [`super::format::AssertionSpec`] is evaluated by
//! exactly one function here. The functions return
//! [`Result<(), AssertionFailure>`] so the runner can accumulate
//! failures across a turn and emit them all rather than aborting on
//! the first.
//!
//! Where an asserter overlaps with the existing
//! [`super::super::messaging_eval`] catalog, we delegate to that
//! module's pure function rather than re-implementing — the goal of
//! the scenario layer is to be a *thin orchestrator*, not a parallel
//! assertion universe.

use std::fmt::{self, Display, Formatter};

use super::format::AssertionSpec;
use super::vocabulary_contract::VocabularyContractRegistry;
use super::TurnContext;

/// Outcome of evaluating one [`AssertionSpec`] against a reply.
#[derive(Debug)]
pub struct AssertionFailure {
    pub spec: AssertionSpec,
    pub reason: String,
}

impl Display for AssertionFailure {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} → {}", self.spec, self.reason)
    }
}

/// Run every assertion in `specs` against the turn context and return
/// the collected failures (empty slice ⇒ all asserts passed).
pub fn evaluate_all(
    specs: &[AssertionSpec],
    ctx: &TurnContext<'_>,
    vocab: &VocabularyContractRegistry,
) -> Vec<AssertionFailure> {
    specs
        .iter()
        .filter_map(|s| evaluate(s, ctx, vocab).err())
        .collect()
}

fn evaluate(
    spec: &AssertionSpec,
    ctx: &TurnContext<'_>,
    vocab: &VocabularyContractRegistry,
) -> Result<(), AssertionFailure> {
    match spec {
        AssertionSpec::ReplyContains { value } => assert_reply_contains(ctx, value, spec),
        AssertionSpec::NoSubstring { values } => assert_no_substring(ctx, values, spec),
        AssertionSpec::DistanceMentioned {
            value_km,
            tolerance_km,
        } => assert_distance_mentioned(ctx, *value_km, *tolerance_km, spec),
        AssertionSpec::ActivityCountMentioned { value, tolerance } => {
            assert_activity_count_mentioned(ctx, *value, *tolerance, spec)
        }
        AssertionSpec::ToolCalled { name, min_calls } => {
            assert_tool_called(ctx, name, *min_calls, spec)
        }
        AssertionSpec::VocabularyContract { coach_id } => {
            assert_vocabulary_contract(ctx, coach_id, vocab, spec)
        }
        AssertionSpec::AnyOf { values } => assert_any_of(ctx, values, spec),
    }
}

fn assert_reply_contains(
    ctx: &TurnContext<'_>,
    needle: &str,
    spec: &AssertionSpec,
) -> Result<(), AssertionFailure> {
    if ctx.reply.to_lowercase().contains(&needle.to_lowercase()) {
        Ok(())
    } else {
        Err(AssertionFailure {
            spec: spec.clone(),
            reason: format!("reply does not contain {needle:?}"),
        })
    }
}

fn assert_no_substring(
    ctx: &TurnContext<'_>,
    forbidden: &[String],
    spec: &AssertionSpec,
) -> Result<(), AssertionFailure> {
    let lower = ctx.reply.to_lowercase();
    let hits: Vec<&String> = forbidden
        .iter()
        .filter(|s| lower.contains(&s.to_lowercase()))
        .collect();
    if hits.is_empty() {
        Ok(())
    } else {
        Err(AssertionFailure {
            spec: spec.clone(),
            reason: format!("reply contains forbidden substring(s): {hits:?}"),
        })
    }
}

fn assert_distance_mentioned(
    ctx: &TurnContext<'_>,
    expected_km: f64,
    tolerance_km: f64,
    spec: &AssertionSpec,
) -> Result<(), AssertionFailure> {
    // Reuse the citation grounding regex catalog from messaging_eval.
    // Distances render as "33.10 km", "33,10 km" (FR decimal), or
    // "33 kilometers". We accept any of those forms.
    let lower = ctx.reply.to_lowercase();
    let pattern = regex::Regex::new(r"(?u)(\d+[\.,]?\d*)\s*(km|kilom[eè]?tres?|kilometers?)")
        .expect("static distance regex compiles");
    for caps in pattern.captures_iter(&lower) {
        if let Some(num) = caps.get(1) {
            let normalized = num.as_str().replace(',', ".");
            if let Ok(value) = normalized.parse::<f64>() {
                if (value - expected_km).abs() <= tolerance_km {
                    return Ok(());
                }
            }
        }
    }
    Err(AssertionFailure {
        spec: spec.clone(),
        reason: format!("no distance within {tolerance_km} km of {expected_km} km found in reply"),
    })
}

fn assert_activity_count_mentioned(
    ctx: &TurnContext<'_>,
    expected: u32,
    tolerance: u32,
    spec: &AssertionSpec,
) -> Result<(), AssertionFailure> {
    let lower = ctx.reply.to_lowercase();
    let pattern =
        regex::Regex::new(r"(?u)(\d+)\s*(activit[ée]s?|sorties?|runs?|rides?|s[ée]ances?)")
            .expect("static activity-count regex compiles");
    for caps in pattern.captures_iter(&lower) {
        if let Some(num) = caps.get(1) {
            if let Ok(value) = num.as_str().parse::<u32>() {
                let delta = value.abs_diff(expected);
                if delta <= tolerance {
                    return Ok(());
                }
            }
        }
    }
    Err(AssertionFailure {
        spec: spec.clone(),
        reason: format!("no activity count within {tolerance} of {expected} found in reply"),
    })
}

fn assert_tool_called(
    ctx: &TurnContext<'_>,
    tool_name: &str,
    min_calls: u32,
    spec: &AssertionSpec,
) -> Result<(), AssertionFailure> {
    let actual = ctx.tools_called.iter().filter(|t| t == &tool_name).count();
    if (actual as u32) >= min_calls {
        Ok(())
    } else {
        Err(AssertionFailure {
            spec: spec.clone(),
            reason: format!(
                "tool {tool_name:?} was called {actual} time(s), expected >= {min_calls}"
            ),
        })
    }
}

fn assert_vocabulary_contract(
    ctx: &TurnContext<'_>,
    coach_id: &str,
    vocab: &VocabularyContractRegistry,
    spec: &AssertionSpec,
) -> Result<(), AssertionFailure> {
    let Some(contract) = vocab.contract_for(coach_id) else {
        return Err(AssertionFailure {
            spec: spec.clone(),
            reason: format!(
                "no vocabulary contract registered for coach {coach_id:?} — declare one in contremaitre"
            ),
        });
    };
    let lower = ctx.reply.to_lowercase();
    let hit = contract
        .terms
        .iter()
        .any(|t| lower.contains(&t.to_lowercase()));
    if hit {
        Ok(())
    } else {
        Err(AssertionFailure {
            spec: spec.clone(),
            reason: format!(
                "reply for coach {coach_id:?} contains none of the declared vocabulary terms ({:?})",
                contract.terms
            ),
        })
    }
}

fn assert_any_of(
    ctx: &TurnContext<'_>,
    values: &[String],
    spec: &AssertionSpec,
) -> Result<(), AssertionFailure> {
    let lower = ctx.reply.to_lowercase();
    if values.iter().any(|v| lower.contains(&v.to_lowercase())) {
        Ok(())
    } else {
        Err(AssertionFailure {
            spec: spec.clone(),
            reason: format!("reply contains none of {values:?}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::vocabulary_contract::{VocabularyContract, VocabularyContractRegistry};
    use super::*;

    fn ctx(reply: &str) -> TurnContext<'_> {
        TurnContext {
            reply,
            tools_called: Vec::new(),
        }
    }

    fn ctx_with_tools<'a>(reply: &'a str, tools: Vec<&str>) -> TurnContext<'a> {
        TurnContext {
            reply,
            tools_called: tools.into_iter().map(str::to_owned).collect(),
        }
    }

    #[test]
    fn reply_contains_substring_passes() {
        let spec = AssertionSpec::ReplyContains {
            value: "Hello".to_owned(),
        };
        let vocab = VocabularyContractRegistry::empty();
        assert!(evaluate(&spec, &ctx("Well, hello world"), &vocab).is_ok());
    }

    #[test]
    fn no_substring_catches_forbidden_word() {
        let spec = AssertionSpec::NoSubstring {
            values: vec!["Medical disclaimer".to_owned()],
        };
        let vocab = VocabularyContractRegistry::empty();
        let err = evaluate(&spec, &ctx("**Medical disclaimer:** ..."), &vocab).unwrap_err();
        assert!(err.reason.contains("Medical disclaimer"));
    }

    #[test]
    fn distance_mentioned_handles_decimal_comma_and_period() {
        let spec = AssertionSpec::DistanceMentioned {
            value_km: 33.10,
            tolerance_km: 0.2,
        };
        let vocab = VocabularyContractRegistry::empty();
        assert!(evaluate(&spec, &ctx("Tu as fait 33,10 km"), &vocab).is_ok());
        assert!(evaluate(&spec, &ctx("You ran 33.10 km total"), &vocab).is_ok());
        assert!(evaluate(&spec, &ctx("Only 25.10 km logged"), &vocab).is_err());
    }

    #[test]
    fn activity_count_mentioned_handles_locale_units() {
        let spec = AssertionSpec::ActivityCountMentioned {
            value: 10,
            tolerance: 0,
        };
        let vocab = VocabularyContractRegistry::empty();
        assert!(evaluate(&spec, &ctx("Tu as fait 10 activités hier"), &vocab).is_ok());
        assert!(evaluate(&spec, &ctx("You logged 10 runs"), &vocab).is_ok());
        assert!(evaluate(&spec, &ctx("Only 7 activités"), &vocab).is_err());
    }

    #[test]
    fn tool_called_with_threshold() {
        let spec = AssertionSpec::ToolCalled {
            name: "get_activities".to_owned(),
            min_calls: 1,
        };
        let vocab = VocabularyContractRegistry::empty();
        assert!(evaluate(
            &spec,
            &ctx_with_tools("reply", vec!["get_activities", "get_stats"]),
            &vocab
        )
        .is_ok());
        assert!(evaluate(&spec, &ctx_with_tools("reply", vec!["get_stats"]), &vocab).is_err());
    }

    #[test]
    fn vocabulary_contract_passes_when_term_present() {
        let mut vocab = VocabularyContractRegistry::empty();
        vocab.insert(
            "strength".to_owned(),
            VocabularyContract {
                terms: vec!["squat".to_owned(), "deload".to_owned(), "rir".to_owned()],
            },
        );
        let spec = AssertionSpec::VocabularyContract {
            coach_id: "strength".to_owned(),
        };
        assert!(evaluate(
            &spec,
            &ctx("Focus on your squat depth and RIR — deload next week."),
            &vocab
        )
        .is_ok());
        assert!(evaluate(
            &spec,
            &ctx("Sleep more, drink water — generic advice."),
            &vocab
        )
        .is_err());
    }

    #[test]
    fn vocabulary_contract_missing_registry_entry_fails_loudly() {
        let vocab = VocabularyContractRegistry::empty();
        let spec = AssertionSpec::VocabularyContract {
            coach_id: "strength".to_owned(),
        };
        let err = evaluate(&spec, &ctx("anything"), &vocab).unwrap_err();
        assert!(err.reason.contains("no vocabulary contract"));
    }

    #[test]
    fn any_of_matches_first_present_term() {
        let spec = AssertionSpec::AnyOf {
            values: vec!["today".to_owned(), "aujourd'hui".to_owned()],
        };
        let vocab = VocabularyContractRegistry::empty();
        assert!(evaluate(&spec, &ctx("Pour aujourd'hui : repos"), &vocab).is_ok());
        assert!(evaluate(&spec, &ctx("Yesterday was hard"), &vocab).is_err());
    }
}
