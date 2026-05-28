// ABOUTME: Curated, user-invokable MCP prompt templates for fitness analysis (prompts/list + prompts/get)
// ABOUTME: Surfaces slash-command-style templated prompts with typed arguments; assembles prompt messages on get
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde_json::{json, Value};
use std::collections::HashMap;
use std::hash::BuildHasher;

/// A single typed argument accepted by a prompt template.
struct PromptArgument {
    /// Argument name as supplied by the client in `prompts/get`.
    name: &'static str,
    /// Human-readable description shown by MCP clients.
    description: &'static str,
    /// Whether the client must supply this argument.
    required: bool,
}

/// A curated analysis prompt template exposed via the MCP `prompts` capability.
///
/// These are user-invokable templates (clients surface them like slash
/// commands), not the server's internal persona/coach system prompts. Each
/// template declares typed arguments and assembles a single user-role prompt
/// message when invoked through `prompts/get`.
struct PromptTemplate {
    /// Stable template identifier used in `prompts/get`.
    name: &'static str,
    /// Human-readable description shown by MCP clients.
    description: &'static str,
    /// Typed arguments the template accepts.
    arguments: &'static [PromptArgument],
}

/// The curated set of analysis templates advertised by the server.
const TEMPLATES: &[PromptTemplate] = &[
    PromptTemplate {
        name: "weekly_training_review",
        description: "Review a week of training: volume, intensity distribution, \
                      standout sessions, and one actionable focus for next week.",
        arguments: &[PromptArgument {
            name: "week",
            description: "Which week to review (e.g. \"this week\", \"last week\", \
                          or an ISO date inside the target week like 2026-05-18).",
            required: false,
        }],
    },
    PromptTemplate {
        name: "race_readiness",
        description: "Assess readiness for an upcoming race given the target date \
                      and distance, including taper guidance and a realistic goal range.",
        arguments: &[
            PromptArgument {
                name: "race_date",
                description: "Race date as an ISO date (e.g. 2026-09-20).",
                required: true,
            },
            PromptArgument {
                name: "distance",
                description: "Race distance (e.g. \"5k\", \"half marathon\", \"42.2km\").",
                required: true,
            },
        ],
    },
    PromptTemplate {
        name: "am_i_overtraining",
        description: "Screen recent training load, recovery, sleep, and HRV trends \
                      for signs of overtraining and recommend adjustments.",
        arguments: &[],
    },
    PromptTemplate {
        name: "compare_training_block",
        description: "Compare two training blocks side by side: volume, intensity, \
                      consistency, and fitness trajectory.",
        arguments: &[
            PromptArgument {
                name: "block_a",
                description: "First training block to compare (e.g. \"last month\", \
                              \"2026-03\", or \"my marathon build\").",
                required: true,
            },
            PromptArgument {
                name: "block_b",
                description: "Second training block to compare against block_a.",
                required: true,
            },
        ],
    },
];

/// Build the `prompts/list` result payload advertising every curated template.
#[must_use]
pub fn list_prompts() -> Value {
    let prompts: Vec<Value> = TEMPLATES
        .iter()
        .map(|template| {
            let arguments: Vec<Value> = template
                .arguments
                .iter()
                .map(|arg| {
                    json!({
                        "name": arg.name,
                        "description": arg.description,
                        "required": arg.required,
                    })
                })
                .collect();
            json!({
                "name": template.name,
                "description": template.description,
                "arguments": arguments,
            })
        })
        .collect();

    json!({ "prompts": prompts })
}

/// Outcome of resolving a `prompts/get` request.
pub enum PromptGetOutcome {
    /// The assembled `prompts/get` result payload.
    Found(Value),
    /// The requested template name does not exist.
    UnknownPrompt(String),
    /// A required argument was missing from the request.
    MissingArgument {
        /// Template that was requested.
        prompt: String,
        /// Name of the missing required argument.
        argument: String,
    },
}

/// Resolve a `prompts/get` request into an assembled prompt message.
///
/// `name` is the requested template identifier and `arguments` carries the
/// client-supplied argument values keyed by argument name.
#[must_use]
pub fn get_prompt<S: BuildHasher>(
    name: &str,
    arguments: &HashMap<String, String, S>,
) -> PromptGetOutcome {
    let Some(template) = TEMPLATES.iter().find(|t| t.name == name) else {
        return PromptGetOutcome::UnknownPrompt(name.to_owned());
    };

    for arg in template.arguments {
        if arg.required && !arguments.contains_key(arg.name) {
            return PromptGetOutcome::MissingArgument {
                prompt: name.to_owned(),
                argument: arg.name.to_owned(),
            };
        }
    }

    let body = render_body(name, arguments);

    PromptGetOutcome::Found(json!({
        "description": template.description,
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": body,
                }
            }
        ]
    }))
}

/// Assemble the prompt body for a template name from supplied argument values.
///
/// The caller has already validated that `name` matches a known template and
/// that every required argument is present.
fn render_body<S: BuildHasher>(name: &str, arguments: &HashMap<String, String, S>) -> String {
    match name {
        "weekly_training_review" => {
            let week = arg_or(arguments, "week", "the most recent completed week");
            format!(
                "Review my training for {week}. Pull the relevant activities first, then analyse:\n\
                 - total volume (distance and duration) versus the prior week\n\
                 - intensity distribution (easy / moderate / hard) and whether it is balanced\n\
                 - the two or three standout sessions and what made them notable\n\
                 - any missed or unusually short sessions and the likely impact\n\n\
                 Finish with one concrete, actionable focus for next week."
            )
        }
        "race_readiness" => {
            let race_date = arg_or(arguments, "race_date", "the supplied race date");
            let distance = arg_or(arguments, "distance", "the supplied distance");
            format!(
                "Assess my readiness for a {distance} race on {race_date}. Review my recent \
                 training load, long sessions, and race-specific workouts, then cover:\n\
                 - whether my current fitness supports the goal distance\n\
                 - the key sessions still worth doing before race day\n\
                 - a taper plan for the final two weeks\n\
                 - a realistic goal-time or goal-effort range with the assumptions behind it.\n\n\
                 Flag any gaps in the data that would change the assessment."
            )
        }
        "compare_training_block" => {
            let block_a = arg_or(arguments, "block_a", "the first block");
            let block_b = arg_or(arguments, "block_b", "the second block");
            format!(
                "Compare two training blocks: \"{block_a}\" and \"{block_b}\". Pull the \
                 activities for each block, then contrast them on:\n\
                 - total volume and weekly consistency\n\
                 - intensity distribution and the balance of easy versus hard work\n\
                 - the trajectory of fitness or performance across each block\n\
                 - injury, illness, or missed-session disruptions\n\n\
                 Summarise which block was more effective for building fitness and why, \
                 and what you would carry forward into the next block."
            )
        }
        // "am_i_overtraining" takes no arguments; the catch-all also covers any
        // name that reaches here only after `get_prompt` validated it against
        // TEMPLATES, so it is always a known arg-free template.
        _ => "Screen me for signs of overtraining. Review my recent training load trend, \
              recovery metrics, sleep, and HRV where available, then report:\n\
              - whether acute load is outpacing chronic load (and by how much)\n\
              - any decline in recovery, sleep quality, or HRV that lines up with hard training\n\
              - resting heart rate or mood/effort signals that corroborate or contradict the load picture\n\n\
              Conclude with a clear verdict (on track / caution / back off) and the specific \
              adjustments you recommend for the next 7–10 days."
            .to_owned(),
    }
}

/// Resolve an optional argument to a trimmed value or a stated default phrase.
fn arg_or<'a, S: BuildHasher>(
    arguments: &'a HashMap<String, String, S>,
    key: &str,
    default: &'a str,
) -> &'a str {
    arguments
        .get(key)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
}
