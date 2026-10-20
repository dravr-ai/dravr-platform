// ABOUTME: On-disk credential cache for pierre-cli's gcloud-style remote admin commands
// ABOUTME: Stores the device-grant access token + server URL at ~/.pierre/credentials.json (0600)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Persistent login state for the authenticated admin CLI.
//!
//! `pierre-cli auth login` writes the device-grant access token here; the
//! remote admin subcommands (`strava-pool`, …) read it. The file holds a
//! bearer token, so it is created with `0600` permissions and never logged.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;

use pierre_core::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};

/// Directory under the user's home that holds pierre-cli state.
const CONFIG_DIR: &str = ".pierre";
/// Credential file name inside [`CONFIG_DIR`].
const CREDENTIALS_FILE: &str = "credentials.json";

/// Cached login state produced by `pierre-cli auth login`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedCredentials {
    /// Base URL of the pierre-server this token authenticates against.
    pub server: String,
    /// Bearer access token minted by the device-grant approval.
    pub access_token: String,
    /// Token type; always `Bearer` for admin tokens.
    #[serde(default = "default_token_type")]
    pub token_type: String,
    /// Expiry as epoch seconds. `None` for super-admin tokens that never expire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// Identity that approved the login (admin email / service name), for display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
}

fn default_token_type() -> String {
    "Bearer".to_owned()
}

impl CachedCredentials {
    /// Absolute path to the credential file: `~/.pierre/credentials.json`.
    ///
    /// # Errors
    /// Returns an error if the user's home directory cannot be determined.
    pub fn path() -> AppResult<PathBuf> {
        let home = dirs::home_dir()
            .ok_or_else(|| AppError::config("Cannot determine home directory for ~/.pierre"))?;
        Ok(home.join(CONFIG_DIR).join(CREDENTIALS_FILE))
    }

    /// Load cached credentials, or `None` if the user has never logged in.
    ///
    /// # Errors
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn load() -> AppResult<Option<Self>> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path)
            .map_err(|e| AppError::internal(format!("Failed to read {}: {e}", path.display())))?;
        let creds: Self = serde_json::from_str(&raw).map_err(|e| {
            AppError::internal(format!(
                "Corrupt credential cache at {} ({e}). Run `pierre-cli auth login` again.",
                path.display()
            ))
        })?;
        Ok(Some(creds))
    }

    /// Load credentials, erroring with an actionable message if absent or expired.
    ///
    /// The remote admin subcommands call this so an unauthenticated invocation
    /// fails fast with "run auth login" instead of a raw 401 later.
    ///
    /// # Errors
    /// Returns [`AppError::auth_required`] when there is no valid cached login.
    pub fn require(now_epoch: i64) -> AppResult<Self> {
        let creds = Self::load()?.ok_or_else(|| {
            AppError::auth_invalid("Not logged in. Run `pierre-cli auth login --server <url>`.")
        })?;
        if let Some(exp) = creds.expires_at {
            if exp <= now_epoch {
                return Err(AppError::auth_expired());
            }
        }
        Ok(creds)
    }

    /// Persist credentials to `~/.pierre/credentials.json` with `0600` perms.
    ///
    /// Written whole to a sibling file created owner-only, flushed, then
    /// renamed over the old one: the token is never readable by anyone else,
    /// not even between a create and a `chmod`, and a crash mid-write leaves
    /// the previous credentials rather than a truncated file.
    ///
    /// # Errors
    /// Returns an error if the directory or file cannot be created/written.
    pub fn save(&self) -> AppResult<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                AppError::internal(format!("Failed to create {}: {e}", parent.display()))
            })?;
        }
        let body = serde_json::to_string_pretty(self)
            .map_err(|e| AppError::internal(format!("Failed to serialize credentials: {e}")))?;
        write_owner_only(&path, body.as_bytes())
    }

    /// Delete the credential file (`pierre-cli auth logout`). Absent file is a no-op.
    ///
    /// # Errors
    /// Returns an error if the file exists but cannot be removed.
    pub fn clear() -> AppResult<bool> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(false);
        }
        fs::remove_file(&path)
            .map_err(|e| AppError::internal(format!("Failed to remove {}: {e}", path.display())))?;
        Ok(true)
    }
}

/// Write `body` to `path` atomically, the file owner-read/write (`0600`) from
/// the moment it exists. Non-Unix platforms have no mode bits; the rename
/// still makes the write atomic there.
fn write_owner_only(path: &Path, body: &[u8]) -> AppResult<()> {
    let failed = |action: &str, target: &Path, e: io::Error| {
        AppError::internal(format!("Failed to {action} {}: {e}", target.display()))
    };
    let staging = path.with_extension(format!("json.tmp-{}", process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&staging).and_then(|mut file| {
        file.write_all(body)?;
        file.sync_all()
    });
    if let Err(e) = written {
        let _ = fs::remove_file(&staging);
        return Err(failed("write", &staging, e));
    }
    fs::rename(&staging, path).map_err(|e| {
        let _ = fs::remove_file(&staging);
        failed("replace", path, e)
    })
}
