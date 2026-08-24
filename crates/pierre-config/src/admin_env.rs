// ABOUTME: Reads the catalog's ConfigLayer env bindings into typed, range-validated config pins
// ABOUTME: One capture at startup — env is immutable at runtime, and an unparseable pin fails the boot

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Environment pins for admin config parameters.
//!
//! Every parameter in the catalog may declare an [`EnvBinding`]. The ones
//! owned by the config layer ([`EnvSource::ConfigLayer`]) are read here,
//! parsed to their declared [`ConfigDataType`], and validated against the
//! same range rules the admin API enforces — so a pin can never install a
//! value a `PUT /api/admin/config` would have rejected.
//!
//! Capture happens once, at service construction: the process environment
//! does not change under a running server, and re-reading it per lookup
//! would make config resolution depend on ambient state.
//!
//! A variable that is set but unparseable is an **error**, never a silent
//! fallback. The whole point of this module is that `QUOTA_DAILY_MESSAGE_CAP`
//! stopped being a name the catalog advertised and nothing read; a typo that
//! degraded back to "ignored" would reintroduce exactly that failure.

use std::collections::HashMap;
use std::env;
use std::hash::BuildHasher;

use serde_json::Value;

use crate::admin_definitions::ParameterDefinition;
use crate::admin_types::{validate_parameter_value, ConfigDataType};

/// A set environment variable that could not become a config value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvConfigError {
    /// Config key the variable binds to (e.g. `usage_quotas.daily_message_cap`).
    pub key: String,
    /// Environment variable name that was set.
    pub env_variable: String,
    /// The raw value read from the environment.
    pub raw: String,
    /// Why it was rejected — a parse failure or a range violation.
    pub message: String,
}

impl EnvConfigError {
    /// One-line operator-facing rendering used in boot failures and logs.
    #[must_use]
    pub fn describe(&self) -> String {
        let Self {
            key,
            env_variable,
            raw,
            message,
        } = self;
        format!("{env_variable}=\"{raw}\" (binds {key}): {message}")
    }
}

/// The environment pins in effect for this process.
///
/// Layered between per-tenant overrides and system-wide stored overrides:
/// a pin is a deploy-time, fleet-wide decision, so it beats a runtime
/// admin edit at the same (global) scope, while a tenant or per-user
/// exemption still beats the pin. This mirrors how `GUARDIAN_*` env
/// overrides sit above the persisted guardian document.
#[derive(Debug, Clone, Default)]
pub struct EnvConfigPins {
    values: HashMap<String, Value>,
}

impl EnvConfigPins {
    /// Read every [`EnvSource::ConfigLayer`] binding that is set in the
    /// environment, returning the usable pins alongside every rejection.
    ///
    /// Rejections are returned rather than logged-and-dropped so the caller
    /// decides the policy: the server fails its boot, while tools that only
    /// inspect configuration can report them.
    #[must_use]
    pub fn capture<S: BuildHasher>(
        definitions: &HashMap<String, ParameterDefinition, S>,
    ) -> (Self, Vec<EnvConfigError>) {
        Self::capture_from(definitions, |name| env::var(name).ok())
    }

    /// [`Self::capture`] against an explicit variable source.
    ///
    /// The process environment is the only source in production; taking the
    /// lookup as a parameter lets the parsing and validation rules be
    /// exercised without mutating global state, which is not safe to do from
    /// tests that run in parallel.
    #[must_use]
    pub fn capture_from<S: BuildHasher, F: Fn(&str) -> Option<String>>(
        definitions: &HashMap<String, ParameterDefinition, S>,
        lookup: F,
    ) -> (Self, Vec<EnvConfigError>) {
        let mut values = HashMap::new();
        let mut errors = Vec::new();

        for def in definitions.values() {
            let Some(binding) = def.env.as_ref().filter(|b| b.is_config_layer()) else {
                continue;
            };
            let Some(raw) = lookup(&binding.name) else {
                continue;
            };

            match parse_env_value(def, &raw) {
                Ok(value) => {
                    values.insert(def.key.clone(), value);
                }
                Err(message) => errors.push(EnvConfigError {
                    key: def.key.clone(),
                    env_variable: binding.name.clone(),
                    raw,
                    message,
                }),
            }
        }

        errors.sort_by(|a, b| a.env_variable.cmp(&b.env_variable));
        (Self { values }, errors)
    }

    /// The pinned value for `key`, if the environment pins it.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    /// Whether no pin is in effect.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// How many keys are pinned.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }
}

/// Parse a raw environment string into the parameter's declared type, then
/// range-validate it exactly as the admin API would.
///
/// Integers and floats accept `_` as a digit separator so a token budget
/// can be written `2_000_000` in `.envrc` the way it is in Rust.
fn parse_env_value(def: &ParameterDefinition, raw: &str) -> Result<Value, String> {
    let trimmed = raw.trim();

    // An empty assignment is a mistake, not a value. Reporting it beats
    // treating the variable as unset, which is the silent-no-op this
    // module exists to remove.
    if trimmed.is_empty() && def.data_type != ConfigDataType::String {
        return Err("variable is set but empty".to_owned());
    }

    let value = match def.data_type {
        ConfigDataType::Integer => {
            let digits = trimmed.replace('_', "");
            let parsed: i64 = digits
                .parse()
                .map_err(|_| format!("expected an integer, got \"{trimmed}\""))?;
            Value::from(parsed)
        }
        ConfigDataType::Float => {
            let digits = trimmed.replace('_', "");
            let parsed: f64 = digits
                .parse()
                .map_err(|_| format!("expected a number, got \"{trimmed}\""))?;
            Value::from(parsed)
        }
        ConfigDataType::Boolean => match trimmed.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Value::Bool(true),
            "false" | "0" | "no" | "off" => Value::Bool(false),
            other => {
                return Err(format!(
                    "expected a boolean (true/false, 1/0, yes/no, on/off), got \"{other}\""
                ))
            }
        },
        ConfigDataType::String => Value::String(raw.to_owned()),
        ConfigDataType::Enum => Value::String(trimmed.to_owned()),
    };

    validate_parameter_value(def, &value).map_err(|e| e.message)?;
    Ok(value)
}
