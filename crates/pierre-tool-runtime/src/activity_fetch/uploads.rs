// ABOUTME: The athlete's uploaded .fit activities folded into every activity read the agent's tools make
// ABOUTME: A local read of the athlete's own rows: no connection, no sync to record, nothing written back

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Uploaded activities in the agent's reads (carnet#818).
//!
//! An uploaded workout is cached under the `upload` key and has no
//! connection behind it, so neither the provider election nor the fold of
//! the athlete's other connections ever reaches it. This module is the one
//! place those reads add it: beside the elected provider's window, beside the
//! connections that stood in for a missing one, and on its own for an athlete
//! whose only activities are uploads. It reads through [`UploadProvider`],
//! the same reader the authentication chokepoint hands out for the `upload`
//! key, and never through the live-fetch path: an upload is not a sync, so
//! nothing here records freshness or writes the cache.

use pierre_core::constants::oauth_providers::UPLOAD;
use pierre_core::models::{Activity, TenantId};
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::upload_provider::UploadProvider;
use pierre_providers::CoreFitnessProvider;
use std::sync::Arc;
use tracing::warn;

use crate::context::ToolExecutionContext;

/// The reader of the athlete's uploads in the context's tenant, or `None`
/// for a call made outside a tenant, which holds none.
fn reader(context: &ToolExecutionContext) -> Option<UploadProvider> {
    let tenant = TenantId::from_uuid(context.tenant_id?);
    let repos = context.resources.repos();
    Some(UploadProvider::new(
        Arc::clone(&repos.activity_cache),
        Arc::clone(&repos.uploaded_activity_files),
        context.user_id,
        tenant,
    ))
}

/// The athlete's uploaded activities inside `params`' window, newest first.
///
/// Best effort, like every other source folded into a read: a failed read is
/// logged and contributes nothing.
pub async fn uploaded_window(
    context: &ToolExecutionContext,
    params: &ActivityQueryParams,
) -> Vec<Activity> {
    let Some(reader) = reader(context) else {
        return Vec::new();
    };
    match reader.get_activities_with_params(params).await {
        Ok(activities) => activities,
        Err(e) => {
            warn!(
                user_id = %context.user_id,
                provider = UPLOAD,
                error = %e,
                "uploaded activities could not be read; the window is served without them"
            );
            Vec::new()
        }
    }
}

/// Whether the athlete holds any uploaded activity file in the context's
/// tenant: an athlete with no connection still has these to answer from.
pub async fn holds_uploads(context: &ToolExecutionContext) -> bool {
    let Some(tenant) = context.tenant_id.map(TenantId::from_uuid) else {
        return false;
    };
    context
        .resources
        .repos()
        .uploaded_activity_files
        .has_uploaded_files(&tenant, context.user_id)
        .await
        .unwrap_or_else(|e| {
            warn!(user_id = %context.user_id, error = %e, "uploaded files lookup failed");
            false
        })
}
