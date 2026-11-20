// ABOUTME: Which role lines the OAuth consent screen shows the signed-in user: the coach line, the admin line
// ABOUTME: Read live from the one coach rule and the admin role, as the tool lists are (carnet#1511)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::User;
use pierre_services::coach_roster::{roster_access, RosterAccess};
use tracing::warn;

use super::pages::ConsentRole;
use super::{OAuth2Context, OAuth2Routes};

impl OAuth2Routes {
    /// The role lines `user` is shown on the consent screen: the coach line
    /// for a human coach by the one coach rule (coach access and an active
    /// group), the admin line for an admin. The tool lists read the same two
    /// facts live, so the screen says what the client will be offered.
    ///
    /// A coach lookup that fails shows no coach line, and the screen still
    /// renders: the line describes the offer, it does not grant it.
    pub(super) async fn consent_role(context: &OAuth2Context, user: &User) -> ConsentRole {
        let coach = match roster_access(context.database.repositories(), user.id).await {
            Ok(access) => matches!(access, RosterAccess::Granted(_)),
            Err(e) => {
                warn!(user_id = %user.id, "consent screen: the coach lookup failed: {e}");
                false
            }
        };
        ConsentRole {
            coach,
            admin: user.role.is_admin(),
        }
    }
}
