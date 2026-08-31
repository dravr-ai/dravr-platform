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
        AssertionSpec::ReplyLanguage { locale } => {
            assert_reply_language(ctx, locale.as_deref(), spec)
        }
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

/// The floor below which a reply is refused without consulting whatlang.
///
/// Mirrors `chat_pipeline::turn_service::detect_turn_locale`'s own floor, so
/// the test and the code under test agree on when a language claim is
/// meaningful at all. It is a fast reject with a better error message, NOT
/// the real gate: whatlang stays unreliable well above 12 characters, and
/// `Info::is_reliable` is what actually refuses that band. See the envelope
/// documented on [`assert_reply_language`].
const MIN_JUDGEABLE_CHARS: usize = 12;

/// Assert the reply is WRITTEN IN the expected language.
///
/// Fails — rather than passes — when the reply is too short or too ambiguous
/// to judge. A language assertion that quietly succeeds on a reply it could
/// not read is worse than no assertion: it reports coverage the suite does not
/// have, which is exactly how the five `*_fr.yaml` scenarios came to look like
/// they were guarding the language when they were only matching substrings.
///
/// **Operating envelope**, measured across the five shipped locales at real
/// reply lengths: at roughly 130 characters and up whatlang returns the right
/// language at confidence 1.00 and `is_reliable()`, in every locale. Below
/// about 60 it is noise — plain English scores as Danish, French as Catalan —
/// and `is_reliable()` correctly refuses all of it. There is no configuration
/// that rescues the short band: restricting the detector to the five platform
/// languages moves the wrong answers around (English reads as French instead
/// of Danish) without making one verdict trustworthy, so this deliberately
/// runs the plain detector and lets the reliability flag do the gating.
/// A turn that cannot produce ~130 characters of prose belongs in
/// `skip_language_check`, not in a weakened assertion.
fn assert_reply_language(
    ctx: &TurnContext<'_>,
    expected: Option<&str>,
    spec: &AssertionSpec,
) -> Result<(), AssertionFailure> {
    let expected = expected.unwrap_or(ctx.locale);
    let reply = ctx.reply.trim();

    let fail = |reason: String| {
        Err(AssertionFailure {
            spec: spec.clone(),
            reason,
        })
    };

    if reply.chars().count() < MIN_JUDGEABLE_CHARS {
        return fail(format!(
            "reply is {} chars, below the {MIN_JUDGEABLE_CHARS}-char floor where a language \
             verdict means anything. Every turn is language-checked against its run locale; \
             if this turn genuinely cannot produce prose, set `skip_language_check: true` on \
             it — do NOT set it on a turn that answered in the wrong language",
            reply.chars().count()
        ));
    }

    let Some(info) = whatlang::detect(reply) else {
        return fail(format!("could not detect any language in reply: {reply:?}"));
    };

    let detected = match info.lang() {
        whatlang::Lang::Fra => "fr",
        whatlang::Lang::Eng => "en",
        whatlang::Lang::Spa => "es",
        whatlang::Lang::Deu => "de",
        whatlang::Lang::Por => "pt",
        other => {
            return fail(format!(
                "reply detected as {other:?}, which is not a language the platform speaks; \
                 expected {expected}"
            ))
        }
    };

    if detected != expected {
        return fail(format!(
            "reply is written in {detected}, expected {expected} (confidence {:.2}). \
             This is the carnet#159 shape: the turn resolved one language and the coach \
             answered in another. Every turn is checked against its scenario's `locales:` \
             whether or not it declares an assertion, so this fires on turns whose YAML \
             looks empty. First 160 chars: {:?}",
            info.confidence(),
            reply.chars().take(160).collect::<String>()
        ));
    }

    // A correct-language verdict that whatlang itself calls unreliable is not
    // evidence. Report it rather than bank it.
    if !info.is_reliable() {
        return fail(format!(
            "reply reads as {detected} (the expected language) but whatlang rates the verdict \
             unreliable at confidence {:.2}, so this turn proves nothing about the language. \
             Replies get reliable below roughly 130 chars; this one is {}. If the turn cannot \
             produce prose, set `skip_language_check: true` on it",
            info.confidence(),
            reply.chars().count()
        ));
    }

    Ok(())
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
            locale: "fr",
        }
    }

    fn ctx_with_tools<'a>(reply: &'a str, tools: Vec<&str>) -> TurnContext<'a> {
        TurnContext {
            reply,
            tools_called: tools.into_iter().map(str::to_owned).collect(),
            locale: "fr",
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

    /// The carnet#159 reply fails, and the French one passes.
    ///
    /// The English text here is the actual reply the athlete received on
    /// 2026-08-30, trimmed. It is the fixture the whole assertion exists for.
    #[test]
    fn reply_language_catches_an_english_answer_on_a_french_turn() {
        let vocab = VocabularyContractRegistry::empty();
        let spec = AssertionSpec::ReplyLanguage { locale: None };

        let english = "This combination — rising non-activity strain on WHOOP, poor sleep, \
                       feeling unusually hot at night, and new urinary difficulty — is not \
                       something a cycling load adjustment can address. That needs a clinician, \
                       not a pacing plan, and I'd rather say that plainly than guess.";
        let err = evaluate(&spec, &ctx(english), &vocab).unwrap_err();
        assert!(
            err.reason.contains("written in en, expected fr"),
            "expected a language mismatch, got: {}",
            err.reason
        );
        assert!(err.reason.contains("carnet#159"));

        let french = "Compris — mardi devient repos complet, pas de 40/20. Je gèle tout \
                      ajustement du ramp de charge tant que tu n'as pas vu un médecin pour la \
                      difficulté urinaire, la chaleur nocturne et le sommeil.";
        assert!(evaluate(&spec, &ctx(french), &vocab).is_ok());
    }

    /// A reply too short to judge FAILS rather than passing.
    ///
    /// This is the whole point of the design. An assertion that passed here
    /// would report coverage on every one-word turn in the suite while
    /// verifying nothing — the vacuous-guard failure mode.
    #[test]
    fn reply_language_refuses_to_judge_a_reply_it_cannot_read() {
        let vocab = VocabularyContractRegistry::empty();
        let spec = AssertionSpec::ReplyLanguage { locale: None };

        let err = evaluate(&spec, &ctx("Ok!"), &vocab).unwrap_err();
        assert!(
            err.reason.contains("below the 12-char floor"),
            "a too-short reply must fail loudly, got: {}",
            err.reason
        );
    }

    /// A full-length coaching reply is judged correctly in all five locales.
    ///
    /// Every turn in the suite is graded through this asserter since
    /// carnet#162, so its envelope is load-bearing: if whatlang could not read
    /// ordinary Spanish or Portuguese coaching prose, making the check
    /// automatic would have turned the nightly into a language-detector bug
    /// report. Each string here is the length and register of a real reply.
    #[test]
    fn a_full_length_reply_is_judged_in_every_shipped_locale() {
        let vocab = VocabularyContractRegistry::empty();
        let corpus = [
            (
                "en",
                "Your training load is trending up and recovery has not kept pace, so \
                    tomorrow should be easy rather than another interval session. Keep the \
                    effort conversational and we reassess on Thursday.",
            ),
            (
                "fr",
                "Ta charge est en hausse et ta récupération n'a pas suivi, donc demain \
                    devrait rester facile plutôt qu'une nouvelle séance d'intervalles. Garde \
                    un rythme conversationnel et on réévalue jeudi.",
            ),
            (
                "es",
                "Tu carga de entrenamiento está subiendo y la recuperación no ha seguido el \
                    ritmo, así que mañana debería ser suave en lugar de otra sesión de series. \
                    Mantén un ritmo conversacional.",
            ),
            (
                "de",
                "Deine Trainingsbelastung steigt und die Erholung ist nicht mitgekommen, \
                    deshalb sollte morgen locker bleiben statt einer weiteren \
                    Intervalleinheit. Halte ein Unterhaltungstempo.",
            ),
            (
                "pt",
                "A tua carga de treino está a subir e a recuperação não acompanhou, por \
                    isso amanhã deve ser leve em vez de outra sessão de intervalos. Mantém um \
                    ritmo conversacional.",
            ),
        ];
        let mut misjudged: Vec<String> = Vec::new();
        for (locale, reply) in corpus {
            let ctx = TurnContext {
                reply,
                tools_called: Vec::new(),
                locale,
            };
            let spec = AssertionSpec::ReplyLanguage { locale: None };
            if let Err(e) = evaluate(&spec, &ctx, &vocab) {
                misjudged.push(format!("[{locale}] → {}", e.reason));
            }
        }
        assert!(
            misjudged.is_empty(),
            "the detector must read a normal reply in each shipped locale:\n{}",
            misjudged.join("\n")
        );
    }

    /// The short band is refused, not guessed at — in every locale.
    ///
    /// One sentence is below where whatlang means anything: plain English
    /// scores as Danish, French as Catalan. The assertion must report that as
    /// a failure the author resolves with `skip_language_check`, never as a
    /// pass. A single reply that slipped through here would be coverage the
    /// suite does not have, on the exact turns most likely to be terse.
    #[test]
    fn a_one_sentence_reply_is_refused_rather_than_guessed() {
        let vocab = VocabularyContractRegistry::empty();
        let short = [
            ("en", "Hello — your training week looks steady so far."),
            ("fr", "Voici ta semaine : 50 km au total, rien d'alarmant."),
            ("es", "Esta semana llevas 50 km en total, nada preocupante."),
            ("pt", "Esta semana tens 50 km no total, nada preocupante."),
        ];
        for (locale, reply) in short {
            let ctx = TurnContext {
                reply,
                tools_called: Vec::new(),
                locale,
            };
            let spec = AssertionSpec::ReplyLanguage { locale: None };
            let Err(err) = evaluate(&spec, &ctx, &vocab) else {
                panic!("[{locale}] a one-sentence reply must not produce a verdict: {reply:?}")
            };
            assert!(
                err.reason.contains("skip_language_check")
                    || err.reason.contains("not a language the platform speaks"),
                "[{locale}] the refusal must tell the author what to do: {}",
                err.reason
            );
        }
    }

    /// An explicit `locale:` overrides the turn's own.
    #[test]
    fn reply_language_honors_an_explicit_locale() {
        let vocab = VocabularyContractRegistry::empty();
        // ctx() runs the turn under "fr"; assert English explicitly.
        let spec = AssertionSpec::ReplyLanguage {
            locale: Some("en".to_owned()),
        };
        let english = "Your training load is trending up and recovery has not kept pace, so \
                       tomorrow should be easy rather than another interval session.";
        assert!(evaluate(&spec, &ctx(english), &vocab).is_ok());
    }
}
