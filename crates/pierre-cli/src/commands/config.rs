// ABOUTME: `pierre-cli config` — read and write admin config at global, tenant, or user scope
// ABOUTME: Speaks the admin API over HTTP, so one binary serves a laptop and a deployed environment

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Operator access to runtime configuration.
//!
//! Overrides resolve most-specific-first — per-user, then per-tenant, then
//! the `.envrc` pin, then the system-wide row, then the parameter's default
//! — and every listing reports which of those rungs supplied the value
//! rather than leaving the operator to infer it.
//!
//! `--user` takes an email or a UUID; an email is resolved through the admin
//! API before the write, so a typo fails with "user not found" instead of
//! silently creating an override keyed to nothing.

use std::collections::BTreeMap;

use chrono::Utc;
use clap::Subcommand;
use pierre_core::errors::{AppError, AppResult};
use serde_json::{json, Value};

use pierre_cli::remote::{CachedCredentials, RemoteClient};

use super::user_admin::resolve_user_id;

const CATALOG_PATH: &str = "/api/admin/config/catalog";
const CONFIG_PATH: &str = "/api/admin/config";
const RESET_PATH: &str = "/api/admin/config/reset";

/// `pierre-cli config` — runtime configuration parameters.
#[non_exhaustive]
#[derive(Subcommand)]
pub enum ConfigCommand {
    /// List parameters with their effective value and where it came from
    Show {
        /// Only this category (e.g. `usage_quotas`)
        #[arg(long)]
        category: Option<String>,

        /// Resolve for this tenant
        #[arg(long)]
        tenant: Option<String>,

        /// Resolve for this user (email or id)
        #[arg(long)]
        user: Option<String>,

        /// Only parameters whose value is not the compiled-in default
        #[arg(long)]
        modified: bool,

        /// Output format: table | json
        #[arg(long, default_value = "table")]
        format: String,
    },

    /// Print one parameter's effective value and source
    Get {
        /// Config key, e.g. `usage_quotas.daily_message_cap`
        key: String,

        /// Resolve for this tenant
        #[arg(long)]
        tenant: Option<String>,

        /// Resolve for this user (email or id)
        #[arg(long)]
        user: Option<String>,
    },

    /// Set a parameter at global, tenant, or user scope
    Set {
        /// Config key, e.g. `usage_quotas.daily_message_cap`
        key: String,

        /// New value. Parsed as JSON when possible, else taken as a string
        value: String,

        /// Write the tenant-scoped row instead of the system-wide one
        #[arg(long)]
        tenant: Option<String>,

        /// Write the per-user row (email or id) — the narrowest scope
        #[arg(long)]
        user: Option<String>,

        /// Reason recorded in the config audit log
        #[arg(long)]
        reason: Option<String>,
    },

    /// Remove overrides so the next-broadest scope applies again
    Reset {
        /// Category to reset within (required by the server)
        #[arg(long)]
        category: String,

        /// Specific keys; omit to reset the whole category at this scope
        #[arg(long = "key")]
        keys: Vec<String>,

        /// Clear the tenant-scoped rows
        #[arg(long)]
        tenant: Option<String>,

        /// Clear the per-user rows (email or id)
        #[arg(long)]
        user: Option<String>,

        /// Reason recorded in the config audit log
        #[arg(long)]
        reason: Option<String>,
    },

    /// Show which parameters this server's environment pins
    Env,
}

/// The scope flags resolved into query parameters.
struct Scope {
    tenant: Option<String>,
    user: Option<String>,
}

impl Scope {
    /// Resolve `--tenant` / `--user`, turning a `--user` email into its id.
    ///
    /// `exclusive` is set for writes: a stored row belongs to exactly one
    /// scope, so naming both is a mistake. Reads allow both, where the pair
    /// means "what this user sees inside this tenant" — the same question
    /// enforcement asks.
    async fn resolve(
        client: &RemoteClient,
        tenant: Option<String>,
        user: Option<String>,
        exclusive: bool,
    ) -> AppResult<Self> {
        if exclusive && tenant.is_some() && user.is_some() {
            return Err(AppError::invalid_input(
                "Pass --tenant or --user, not both — an override belongs to one scope",
            ));
        }
        let user = match user {
            Some(selector) => Some(resolve_user_id(client, &selector).await?),
            None => None,
        };
        Ok(Self { tenant, user })
    }

    /// Query string for the admin API, empty at global scope.
    fn query(&self) -> String {
        let mut parts = Vec::new();
        if let Some(t) = &self.tenant {
            parts.push(format!("tenant_id={t}"));
        }
        if let Some(u) = &self.user {
            parts.push(format!("user_id={u}"));
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("?{}", parts.join("&"))
        }
    }

    /// Operator-facing name of the scope in play.
    fn label(&self) -> String {
        match (&self.tenant, &self.user) {
            (Some(t), Some(u)) => format!("user {u} in tenant {t}"),
            (Some(t), None) => format!("tenant {t}"),
            (None, Some(u)) => format!("user {u}"),
            (None, None) => "global".to_owned(),
        }
    }
}

fn client() -> AppResult<RemoteClient> {
    let creds = CachedCredentials::require(Utc::now().timestamp())?;
    RemoteClient::from_cached(&creds)
}

/// One catalog row, flattened out of the nested category response.
struct Param {
    key: String,
    value: Value,
    default: Value,
    source: String,
    env_variable: Option<String>,
    env_pinned: bool,
    units: Option<String>,
}

fn flatten_catalog(body: &Value, category: Option<&str>) -> Vec<Param> {
    let mut rows = Vec::new();
    let categories = body
        .get("data")
        .and_then(|d| d.get("categories"))
        .or_else(|| body.get("categories"))
        .and_then(Value::as_array);

    for cat in categories.into_iter().flatten() {
        let name = cat.get("name").and_then(Value::as_str).unwrap_or_default();
        if category.is_some_and(|want| want != name) {
            continue;
        }
        let params = cat.get("parameters").and_then(Value::as_array);
        for p in params.into_iter().flatten() {
            rows.push(Param {
                key: p
                    .get("key")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                value: p.get("current_value").cloned().unwrap_or(Value::Null),
                default: p.get("default_value").cloned().unwrap_or(Value::Null),
                source: p
                    .get("value_source")
                    .and_then(Value::as_str)
                    .unwrap_or("default")
                    .to_owned(),
                env_variable: p
                    .get("env_variable")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                env_pinned: p
                    .get("env_pinned")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                units: p
                    .get("units")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
            });
        }
    }
    rows.sort_by(|a, b| a.key.cmp(&b.key));
    rows
}

fn render(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), ToOwned::to_owned)
}

fn print_table(rows: &[Param]) {
    println!("{:<48}  {:>14}  {:<8}  UNITS", "KEY", "VALUE", "SOURCE");
    for r in rows {
        println!(
            "{:<48}  {:>14}  {:<8}  {}",
            r.key,
            render(&r.value),
            r.source,
            r.units.clone().unwrap_or_default()
        );
    }
}

/// Dispatch a `config` subcommand.
///
/// # Errors
///
/// Returns an error if not logged in, the server rejects the request, or a
/// scope flag names a user that does not exist.
pub async fn dispatch(command: ConfigCommand) -> AppResult<()> {
    let client = client()?;
    match command {
        ConfigCommand::Show {
            category,
            tenant,
            user,
            modified,
            format,
        } => {
            let scope = Scope::resolve(&client, tenant, user, false).await?;
            let body = client
                .get_json(&format!("{CATALOG_PATH}{}", scope.query()))
                .await?;
            let mut rows = flatten_catalog(&body, category.as_deref());
            if modified {
                rows.retain(|r| r.source != "default");
            }
            if format == "json" {
                let out: BTreeMap<&str, Value> = rows
                    .iter()
                    .map(|r| {
                        (
                            r.key.as_str(),
                            json!({"value": r.value, "source": r.source, "default": r.default}),
                        )
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                println!("scope: {}", scope.label());
                print_table(&rows);
            }
            Ok(())
        }

        ConfigCommand::Get { key, tenant, user } => {
            let scope = Scope::resolve(&client, tenant, user, false).await?;
            let body = client
                .get_json(&format!("{CATALOG_PATH}{}", scope.query()))
                .await?;
            let rows = flatten_catalog(&body, None);
            let row = rows
                .iter()
                .find(|r| r.key == key)
                .ok_or_else(|| AppError::not_found(format!("Config key '{key}'")))?;

            println!("{key} = {}", render(&row.value));
            println!("  scope   {}", scope.label());
            println!("  source  {}", row.source);
            println!("  default {}", render(&row.default));
            if let Some(var) = &row.env_variable {
                let state = if row.env_pinned { "set" } else { "not set" };
                println!("  env     {var} ({state})");
            }
            Ok(())
        }

        ConfigCommand::Set {
            key,
            value,
            tenant,
            user,
            reason,
        } => {
            let scope = Scope::resolve(&client, tenant, user, true).await?;
            // A bare `500` is a number and `true` a boolean; anything that is
            // not valid JSON is the operator typing a plain string.
            let parsed: Value =
                serde_json::from_str(&value).unwrap_or_else(|_| Value::String(value.clone()));

            let payload = json!({
                "parameters": { key.clone(): parsed },
                "reason": reason.unwrap_or_else(|| format!("set via pierre-cli ({})", scope.label())),
            });
            let body = client
                .put_json(&format!("{CONFIG_PATH}{}", scope.query()), &payload)
                .await?;

            let data = body.get("data").unwrap_or(&body);
            let errors = data.get("validation_errors").and_then(Value::as_array);
            if let Some(errs) = errors.filter(|e| !e.is_empty()) {
                for e in errs {
                    let param = e.get("parameter").and_then(Value::as_str).unwrap_or(&key);
                    let msg = e
                        .get("error")
                        .or_else(|| e.get("message"))
                        .and_then(Value::as_str)
                        .unwrap_or("rejected");
                    eprintln!("rejected: {param}: {msg}");
                }
                return Err(AppError::invalid_input(format!(
                    "Server rejected the value for {key}"
                )));
            }

            println!("Set {key} at {} scope", scope.label());

            // A stored global row that an environment pin outranks is saved
            // but not read back. Without this the write looks like a no-op.
            let shadowed: Vec<&str> = data
                .get("shadowed_by_env")
                .and_then(Value::as_array)
                .map(|keys| keys.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            for shadowed_key in shadowed {
                println!(
                    "note: {shadowed_key} is pinned by an environment variable, which outranks \
                     the system-wide scope — the stored value applies once the variable is unset"
                );
                println!("      `pierre-cli config env` shows what this server pins");
            }

            if data
                .get("requires_restart")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                println!("note: this parameter takes effect on the next server restart");
            }
            Ok(())
        }

        ConfigCommand::Reset {
            category,
            keys,
            tenant,
            user,
            reason,
        } => {
            let scope = Scope::resolve(&client, tenant, user, true).await?;
            let mut payload = json!({ "category": category });
            if !keys.is_empty() {
                payload["keys"] = json!(keys);
            }
            if let Some(r) = reason {
                payload["reason"] = json!(r);
            }
            let body = client
                .post_json(&format!("{RESET_PATH}{}", scope.query()), &payload)
                .await?;
            let count = body
                .get("data")
                .and_then(|d| d.get("reset_count"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            println!("Cleared {count} override(s) at {} scope", scope.label());
            Ok(())
        }

        ConfigCommand::Env => {
            let body = client.get_json(CATALOG_PATH).await?;
            let rows: Vec<Param> = flatten_catalog(&body, None)
                .into_iter()
                .filter(|r| r.env_variable.is_some())
                .collect();

            let (pinned, available): (Vec<&Param>, Vec<&Param>) =
                rows.iter().partition(|r| r.env_pinned);

            println!("Pinned by this server's environment ({}):", pinned.len());
            if pinned.is_empty() {
                println!("  (none)");
            }
            for r in pinned {
                println!(
                    "  {:<38} {:>12}  <- {}",
                    r.key,
                    render(&r.value),
                    r.env_variable.clone().unwrap_or_default()
                );
            }
            println!("\nAvailable but unset ({}):", available.len());
            for r in available {
                println!(
                    "  {:<38} {:>12}  <- {}",
                    r.key,
                    render(&r.default),
                    r.env_variable.clone().unwrap_or_default()
                );
            }
            Ok(())
        }
    }
}
