// ABOUTME: Admin handler for the new-user cohort: each account created in a window and how far it got
// ABOUTME: Super-admins read platform-wide; any other admin reads the members of the tenant it acts in

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension,
};
use chrono::Utc;
use serde::Deserialize;
use tracing::{error, info};

use pierre_core::admin::models::{AdminPermission as AdminPerm, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_services::new_user_cohort::{clamp_window_days, fetch_new_user_cohort};

use super::api_keys::json_response;
use super::types::AdminResponse;
use super::users::token_tenant;
use crate::context::AdminApiContext;

/// Query string of the new-user report.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct NewUsersQuery {
    /// Days back from now; defaulted to 30 and clamped to `1..=180`.
    pub days: Option<i64>,
}

/// Where an admin token reads the new-user cohort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CohortScope {
    /// Every account in every tenant: a super-admin's read, and only theirs.
    Platform,
    /// The members of the tenant the token acts in.
    Tenant(TenantId),
    /// Nowhere: an admin that is not a super-admin and acts in no tenant.
    Refused,
}

/// The scope `token` reads the cohort in. Only a super-admin reads
/// platform-wide; any other admin reads the tenant it acts in, and one acting
/// in none is refused rather than handed every tenant's accounts.
fn cohort_scope(token: &ValidatedAdminToken) -> AppResult<CohortScope> {
    if token.is_super_admin {
        return Ok(CohortScope::Platform);
    }
    Ok(token_tenant(token)?.map_or(CohortScope::Refused, CohortScope::Tenant))
}

/// `GET /admin/analytics/new-users?days=N` — the accounts created in the last
/// `N` days, each with its activation milestones, and the funnel over them.
///
/// A super-admin reads every account; any other admin reads only the members
/// of the tenant it acts in, with every milestone read in that tenant. An
/// admin that is not a super-admin and acts in no tenant is refused: the
/// platform-wide read is the super-admin's alone.
///
/// # Errors
///
/// `ManageUsers` is required; a repository failure answers 500.
pub(crate) async fn handle_new_users(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(query): Query<NewUsersQuery>,
) -> AppResult<impl IntoResponse> {
    if !admin_token
        .permissions
        .has_permission(&AdminPerm::ManageUsers)
    {
        return Ok(json_response(
            AdminResponse {
                success: false,
                message: "Permission denied: ManageUsers required".to_owned(),
                data: None,
            },
            StatusCode::FORBIDDEN,
        ));
    }

    let scope = match cohort_scope(&admin_token)? {
        CohortScope::Platform => None,
        CohortScope::Tenant(tenant) => Some(tenant),
        CohortScope::Refused => {
            return Ok(json_response(
                AdminResponse {
                    success: false,
                    message: "Permission denied: a tenant scope is required".to_owned(),
                    data: None,
                },
                StatusCode::FORBIDDEN,
            ));
        }
    };
    let window_days = clamp_window_days(query.days);

    let cohort = fetch_new_user_cohort(&context.repos, window_days, scope, Utc::now())
        .await
        .inspect_err(|e| error!(error = %e, "Failed to read the new-user cohort"))?;

    let count = cohort.users.len();
    info!(
        window_days,
        count,
        scoped = scope.is_some(),
        token_id = %admin_token.token_id,
        "New-user cohort read"
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("{count} accounts created in the last {window_days} days"),
            data: Some(serde_json::to_value(&cohort).map_err(|e| {
                AppError::internal(format!("Failed to serialize the new-user cohort: {e}"))
            })?),
        },
        StatusCode::OK,
    ))
}

#[cfg(test)]
mod tests {
    use pierre_core::admin::models::{AdminPermissions, ValidatedAdminToken};
    use pierre_core::models::TenantId;

    use super::{cohort_scope, CohortScope};

    fn admin(is_super_admin: bool, tenant: Option<TenantId>) -> ValidatedAdminToken {
        ValidatedAdminToken {
            token_id: "test-token".to_owned(),
            service_name: "test".to_owned(),
            permissions: if is_super_admin {
                AdminPermissions::super_admin()
            } else {
                AdminPermissions::console_admin()
            },
            is_super_admin,
            tenant_id: tenant.map(|t| t.to_string()),
            operator_user_id: None,
            user_info: None,
        }
    }

    #[test]
    fn only_a_super_admin_reads_platform_wide() {
        let tenant = TenantId::generate();
        assert_eq!(
            cohort_scope(&admin(true, None)).unwrap(),
            CohortScope::Platform
        );
        assert_eq!(
            cohort_scope(&admin(true, Some(tenant))).unwrap(),
            CohortScope::Platform,
            "a super-admin acting in a tenant still reads platform-wide"
        );
    }

    #[test]
    fn any_other_admin_reads_its_own_tenant() {
        let tenant = TenantId::generate();
        assert_eq!(
            cohort_scope(&admin(false, Some(tenant))).unwrap(),
            CohortScope::Tenant(tenant)
        );
    }

    #[test]
    fn an_admin_acting_in_no_tenant_is_refused_the_platform_read() {
        assert_eq!(
            cohort_scope(&admin(false, None)).unwrap(),
            CohortScope::Refused,
            "a plain admin with no tenant must not fall through to every tenant's accounts"
        );
    }
}
