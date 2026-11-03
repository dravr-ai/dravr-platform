// ABOUTME: PostgreSQL encryption support: seals, opens and hashes through the shared process keyring
// ABOUTME: The one implementation lives in backends/shared/keyring.rs, so the two backends cannot drift
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use super::PostgresDatabase;
use crate::backends::shared::encryption::HasEncryption;
use crate::backends::shared::keyring::KeyringCell;

impl HasEncryption for PostgresDatabase {
    fn keyring(&self) -> &Arc<KeyringCell> {
        &self.keyring
    }
}
