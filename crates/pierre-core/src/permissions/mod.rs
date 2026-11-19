// ABOUTME: The two roles an account holds, admin and user, and how a role is stored and parsed
// ABOUTME: Parent module of the impersonation session record and the OAuth scope vocabulary
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::errors::AppError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Impersonation system for admins to act as other users
pub mod impersonation;

/// The OAuth scope vocabulary this server publishes and enforces
pub mod scopes;

/// The two roles an account holds.
///
/// An admin holds every permission, impersonation and admin management
/// included (carnet#1567): there is no tier above it, and nothing between it
/// and a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    /// Regular user with access to own data only
    #[default]
    User,
    /// Administrator with full system access, including impersonation and
    /// managing other admins
    Admin,
}

impl UserRole {
    /// Check if this role is admin
    #[must_use]
    pub const fn is_admin(&self) -> bool {
        matches!(self, Self::Admin)
    }

    /// Convert to database string representation
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Admin => "admin",
        }
    }

    /// Parse from database string; anything that is not `admin` reads as a
    /// plain user
    #[must_use]
    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "admin" => Self::Admin,
            _ => Self::User,
        }
    }
}

impl fmt::Display for UserRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for UserRole {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "admin" => Ok(Self::Admin),
            "user" => Ok(Self::User),
            _ => Err(AppError::invalid_input(format!("Invalid user role: {s}"))),
        }
    }
}
