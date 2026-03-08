// ABOUTME: Shared database logic for PostgreSQL and SQLite implementations
// ABOUTME: Eliminates duplication by extracting common business logic
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Enum conversion utilities (`UserTier`, `UserStatus`, `TaskStatus`, etc.)
pub mod enums;

/// Input validation logic (email, tenant ownership, expiration, scopes)
pub mod validation;

/// Model ↔ SQL row conversion helpers (row parsing, struct construction)
pub mod mappers;

/// Encryption/decryption utilities for OAuth tokens and sensitive data
pub mod encryption;

/// Transaction retry patterns (deadlock handling, exponential backoff)
pub mod transactions;
