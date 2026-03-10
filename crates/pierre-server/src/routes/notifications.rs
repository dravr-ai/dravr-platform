// ABOUTME: Route handlers for push notification REST API (device tokens, preferences, feed)
// ABOUTME: Provides endpoints for notification management with multi-tenant isolation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Push notification routes
//!
//! This module handles notification endpoints for device registration,
//! preference management, and notification feed. All endpoints require
//! JWT authentication and enforce tenant-scoped data access.

use std::env;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use tracing::info;
use uuid::Uuid;

use pierre_auth::auth::AuthResult;

use crate::{errors::AppError, mcp::resources::ServerResources, middleware::AuthenticatedUser};
use pierre_core::models::notifications::{
    collapse_notifications, CreateScheduledNotificationParams, CreateScheduledNotificationRequest,
    ListNotificationsQuery, NotificationAnalyticsQuery, NotificationCategory,
    NotificationFeedResponse, NotificationItem, NotificationPreferenceItem,
    NotificationPreferencesResponse, RegisterDeviceTokenRequest, ScheduledNotificationItem,
    UpdateNotificationPreferenceRequest, UpdateScheduledNotificationParams,
    UpdateScheduledNotificationRequest, UpsertNotificationPreferenceParams,
};
use pierre_core::models::TenantId;
use pierre_notifications::constants as notif_constants;
use pierre_notifications::{compute_next_fire_time, validate_cron_expression, NotificationService};

/// Notification routes configuration
pub struct NotificationRoutes;

/// Response for device token registration
#[derive(Debug, Serialize)]
struct DeviceTokenResponse {
    id: Uuid,
    user_id: Uuid,
    tenant_id: TenantId,
    expo_push_token: String,
    platform: String,
    device_name: Option<String>,
    active: bool,
    created_at: String,
}

/// Response for mark-all-read operation
#[derive(Debug, Serialize)]
struct MarkAllReadResponse {
    updated_count: u64,
}

impl NotificationRoutes {
    /// Create all notification routes
    ///
    /// # Endpoints
    ///
    /// - `POST /api/notifications/device` - Register device token
    /// - `GET /api/notifications/device` - List device tokens
    /// - `DELETE /api/notifications/device/:id` - Deactivate device token
    /// - `GET /api/notifications/preferences` - Get notification preferences
    /// - `PUT /api/notifications/preferences` - Update notification preferences
    /// - `GET /api/notifications` - List notifications (feed)
    /// - `GET /api/notifications/unread-count` - Get unread count
    /// - `POST /api/notifications/:id/read` - Mark notification as read
    /// - `POST /api/notifications/:id/opened` - Mark notification as opened
    /// - `POST /api/notifications/:id/dismissed` - Mark notification as dismissed
    /// - `POST /api/notifications/read-all` - Mark all as read
    /// - `DELETE /api/notifications/:id` - Delete notification
    /// - `GET /api/notifications/analytics` - Get notification analytics
    /// - `GET /api/notifications/scheduled` - List scheduled notifications
    /// - `POST /api/notifications/scheduled` - Create scheduled notification
    /// - `PUT /api/notifications/scheduled/{id}` - Update scheduled notification
    /// - `DELETE /api/notifications/scheduled/{id}` - Delete scheduled notification
    pub fn routes(resources: Arc<ServerResources>) -> Router {
        Router::new()
            // Device token management
            .route(
                "/api/notifications/device",
                post(Self::handle_register_device).get(Self::handle_list_devices),
            )
            .route(
                "/api/notifications/device/{id}",
                delete(Self::handle_deactivate_device),
            )
            // Preferences management
            .route(
                "/api/notifications/preferences",
                get(Self::handle_get_preferences).put(Self::handle_update_preferences),
            )
            // Notification feed
            .route("/api/notifications", get(Self::handle_list_notifications))
            .route(
                "/api/notifications/unread-count",
                get(Self::handle_unread_count),
            )
            .route("/api/notifications/{id}/read", post(Self::handle_mark_read))
            .route(
                "/api/notifications/{id}/opened",
                post(Self::handle_mark_opened),
            )
            .route(
                "/api/notifications/{id}/dismissed",
                post(Self::handle_mark_dismissed),
            )
            .route(
                "/api/notifications/read-all",
                post(Self::handle_mark_all_read),
            )
            .route(
                "/api/notifications/{id}",
                delete(Self::handle_delete_notification),
            )
            // Analytics
            .route(
                "/api/notifications/analytics",
                get(Self::handle_get_analytics),
            )
            // Badge sync
            .route(
                "/api/notifications/badge-sync",
                post(Self::handle_badge_sync),
            )
            // Scheduled notifications
            .route(
                "/api/notifications/scheduled",
                get(Self::handle_list_scheduled).post(Self::handle_create_scheduled),
            )
            .route(
                "/api/notifications/scheduled/{id}",
                put(Self::handle_update_scheduled).delete(Self::handle_delete_scheduled),
            )
            .with_state(resources)
    }

    /// Extract tenant ID from authentication result
    fn get_tenant_id(auth: &AuthResult) -> Result<TenantId, AppError> {
        auth.active_tenant_id
            .map(TenantId::from)
            .ok_or_else(|| AppError::auth_invalid("No active tenant in session"))
    }

    /// Get the notification service from resources, returning an error if not initialized
    fn get_service(resources: &ServerResources) -> Result<&NotificationService, AppError> {
        resources
            .notification_service
            .as_deref()
            .ok_or_else(|| AppError::internal("Notification service not initialized"))
    }

    /// Handle POST /api/notifications/device - Register a device token
    async fn handle_register_device(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
        Json(request): Json<RegisterDeviceTokenRequest>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Validate expo push token format
        if !request.expo_push_token.starts_with("ExponentPushToken[")
            && !request.expo_push_token.starts_with("ExpoPushToken[")
        {
            return Err(AppError::invalid_input(
                "Invalid Expo push token format. Expected ExponentPushToken[...] or ExpoPushToken[...]",
            ));
        }

        let service = Self::get_service(&resources)?;
        let token = service
            .upsert_device_token(
                auth.user_id,
                tenant_id,
                &request.expo_push_token,
                request.platform.as_str(),
                request.device_name.as_deref(),
            )
            .await?;

        info!(
            user_id = %auth.user_id,
            tenant_id = %tenant_id.0,
            platform = %token.platform,
            "Device token registered"
        );

        let response = DeviceTokenResponse {
            id: token.id,
            user_id: token.user_id,
            tenant_id: token.tenant_id,
            expo_push_token: token.expo_push_token,
            platform: token.platform.to_string(),
            device_name: token.device_name,
            active: token.active,
            created_at: token.created_at.to_rfc3339(),
        };

        Ok((StatusCode::CREATED, Json(response)).into_response())
    }

    /// Handle GET /api/notifications/device - List active device tokens
    async fn handle_list_devices(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        let service = Self::get_service(&resources)?;
        let tokens = service.get_device_tokens(auth.user_id, tenant_id).await?;

        let response: Vec<DeviceTokenResponse> = tokens
            .into_iter()
            .map(|t| DeviceTokenResponse {
                id: t.id,
                user_id: t.user_id,
                tenant_id: t.tenant_id,
                expo_push_token: t.expo_push_token,
                platform: t.platform.to_string(),
                device_name: t.device_name,
                active: t.active,
                created_at: t.created_at.to_rfc3339(),
            })
            .collect();

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle DELETE /api/notifications/device/:id - Deactivate device token
    async fn handle_deactivate_device(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
        Path(id): Path<Uuid>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        let service = Self::get_service(&resources)?;
        let deactivated = service
            .deactivate_device_token(auth.user_id, tenant_id, id)
            .await?;

        if !deactivated {
            return Err(AppError::not_found("Device token"));
        }

        Ok((StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response())
    }

    /// Handle GET /api/notifications/preferences - Get all preferences
    async fn handle_get_preferences(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        let service = Self::get_service(&resources)?;
        let prefs = service
            .get_notification_preferences(auth.user_id, tenant_id)
            .await?;

        let items: Vec<NotificationPreferenceItem> = prefs
            .into_iter()
            .map(NotificationPreferenceItem::from)
            .collect();

        let response = NotificationPreferencesResponse {
            user_id: auth.user_id,
            tenant_id,
            preferences: items,
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle PUT /api/notifications/preferences - Update a preference
    async fn handle_update_preferences(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
        Json(request): Json<UpdateNotificationPreferenceRequest>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        let enabled = request.enabled.unwrap_or(true);

        // Validate max_per_day range if provided
        if let Some(max) = request.max_per_day {
            if !(0..=1000).contains(&max) {
                return Err(AppError::invalid_input(
                    "max_per_day must be between 0 and 1000",
                ));
            }
        }

        let params = UpsertNotificationPreferenceParams {
            user_id: auth.user_id,
            tenant_id,
            category: request.category.as_str().to_owned(),
            enabled,
            sub_preferences: request.sub_preferences.clone(),
            quiet_hours_start: request.quiet_hours_start.clone(),
            quiet_hours_end: request.quiet_hours_end.clone(),
            timezone: request.timezone.clone(),
            max_per_day: request.max_per_day,
        };

        let service = Self::get_service(&resources)?;
        let pref = service.upsert_notification_preference(&params).await?;

        info!(
            user_id = %auth.user_id,
            category = %request.category,
            enabled = enabled,
            "Notification preference updated"
        );

        let item = NotificationPreferenceItem::from(pref);
        Ok((StatusCode::OK, Json(item)).into_response())
    }

    /// Handle GET /api/notifications - List notifications (feed)
    async fn handle_list_notifications(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
        Query(query): Query<ListNotificationsQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        let limit = query.limit.unwrap_or(20).clamp(1, 100);
        let offset = query.offset.unwrap_or(0);
        let unread_only = query.unread_only.unwrap_or(false);

        // Validate category if provided
        if let Some(ref cat) = query.category {
            if NotificationCategory::from_str_opt(cat).is_none() {
                return Err(AppError::invalid_input(format!(
                    "Invalid category: {cat}. Valid categories: training, recovery, social, coach, achievement, system, ai, reminders"
                )));
            }
        }

        let service = Self::get_service(&resources)?;
        let (notifications, total, unread_count) = service
            .list_notifications(
                auth.user_id,
                tenant_id,
                limit,
                offset,
                query.category.as_deref(),
                unread_only,
            )
            .await?;

        let items: Vec<NotificationItem> = notifications
            .into_iter()
            .map(NotificationItem::from)
            .collect();

        // Collapse consecutive notifications of the same collapsible type
        let collapsed_items = collapse_notifications(items);

        let response = NotificationFeedResponse {
            data: collapsed_items,
            total,
            unread_count,
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle GET /api/notifications/unread-count - Get unread count
    async fn handle_unread_count(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        let service = Self::get_service(&resources)?;
        let count = service.get_unread_count(auth.user_id, tenant_id).await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({"unread_count": count})),
        )
            .into_response())
    }

    /// Handle POST /api/notifications/:id/read - Mark as read
    async fn handle_mark_read(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
        Path(id): Path<Uuid>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        let service = Self::get_service(&resources)?;
        let marked = service
            .mark_notification_read(auth.user_id, tenant_id, id)
            .await?;

        if !marked {
            return Err(AppError::not_found("Notification"));
        }

        Ok((StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response())
    }

    /// Handle POST /api/notifications/read-all - Mark all as read
    async fn handle_mark_all_read(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        let service = Self::get_service(&resources)?;
        let updated = service
            .mark_all_notifications_read(auth.user_id, tenant_id)
            .await?;

        info!(
            user_id = %auth.user_id,
            updated_count = updated,
            "Marked all notifications as read"
        );

        let response = MarkAllReadResponse {
            updated_count: updated,
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle DELETE /api/notifications/:id - Delete notification
    async fn handle_delete_notification(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
        Path(id): Path<Uuid>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        let service = Self::get_service(&resources)?;
        let deleted = service
            .delete_notification(auth.user_id, tenant_id, id)
            .await?;

        if !deleted {
            return Err(AppError::not_found("Notification"));
        }

        Ok((StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response())
    }

    /// Handle POST /api/notifications/:id/opened - Mark notification as opened
    async fn handle_mark_opened(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
        Path(id): Path<Uuid>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        let service = Self::get_service(&resources)?;
        let marked = service
            .mark_notification_opened(auth.user_id, tenant_id, id)
            .await?;

        if !marked {
            return Err(AppError::not_found("Notification"));
        }

        Ok((StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response())
    }

    /// Handle POST /api/notifications/:id/dismissed - Mark notification as dismissed
    async fn handle_mark_dismissed(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
        Path(id): Path<Uuid>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        let service = Self::get_service(&resources)?;
        let marked = service
            .mark_notification_dismissed(auth.user_id, tenant_id, id)
            .await?;

        if !marked {
            return Err(AppError::not_found("Notification"));
        }

        Ok((StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response())
    }

    /// Handle GET /api/notifications/analytics - Get notification analytics
    async fn handle_get_analytics(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
        Query(query): Query<NotificationAnalyticsQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        let since = query
            .since
            .as_deref()
            .map(DateTime::parse_from_rfc3339)
            .transpose()
            .map_err(|e| AppError::invalid_input(format!("Invalid 'since' date: {e}")))?
            .map(|dt| dt.with_timezone(&Utc));

        let until = query
            .until
            .as_deref()
            .map(DateTime::parse_from_rfc3339)
            .transpose()
            .map_err(|e| AppError::invalid_input(format!("Invalid 'until' date: {e}")))?
            .map(|dt| dt.with_timezone(&Utc));

        let service = Self::get_service(&resources)?;
        let analytics = service
            .get_notification_analytics(
                auth.user_id,
                tenant_id,
                since,
                until,
                query.category.as_deref(),
            )
            .await?;

        Ok((StatusCode::OK, Json(analytics)).into_response())
    }

    /// Handle GET /api/notifications/scheduled - List user's scheduled notifications
    async fn handle_list_scheduled(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        let service = Self::get_service(&resources)?;
        let schedules = service
            .list_scheduled_notifications(auth.user_id, tenant_id)
            .await?;

        let items: Vec<ScheduledNotificationItem> = schedules
            .into_iter()
            .map(ScheduledNotificationItem::from)
            .collect();

        Ok((StatusCode::OK, Json(items)).into_response())
    }

    /// Handle POST /api/notifications/scheduled - Create a scheduled notification
    async fn handle_create_scheduled(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
        Json(request): Json<CreateScheduledNotificationRequest>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Enforce per-user schedule cap
        let max_schedules = env::var("NOTIFICATION_MAX_SCHEDULES_PER_USER")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(notif_constants::MAX_SCHEDULES_PER_USER);
        let service = Self::get_service(&resources)?;
        let schedule_count = service
            .count_scheduled_notifications(auth.user_id, tenant_id)
            .await?;
        if schedule_count >= max_schedules {
            return Err(AppError::invalid_input(format!(
                "Maximum of {max_schedules} scheduled notifications per user reached",
            )));
        }

        // Validate cron expression
        validate_cron_expression(&request.schedule_cron).map_err(AppError::invalid_input)?;

        let timezone = request.timezone.as_deref().unwrap_or("UTC");

        // Validate timezone
        timezone
            .parse::<chrono_tz::Tz>()
            .map_err(|_| AppError::invalid_input(format!("Invalid timezone: {timezone}")))?;

        // Compute initial next fire time
        let next_fire_at = compute_next_fire_time(&request.schedule_cron, timezone, Utc::now())
            .ok_or_else(|| {
                AppError::invalid_input("Could not compute next fire time from cron expression")
            })?;

        let params = CreateScheduledNotificationParams {
            user_id: auth.user_id,
            tenant_id,
            notification_type: request.notification_type,
            schedule_cron: request.schedule_cron,
            timezone: timezone.to_owned(),
            next_fire_at,
        };

        let scheduled = service.create_scheduled_notification(&params).await?;

        info!(
            user_id = %auth.user_id,
            notification_type = %scheduled.notification_type,
            schedule_cron = %scheduled.schedule_cron,
            "Scheduled notification created"
        );

        let item = ScheduledNotificationItem::from(scheduled);
        Ok((StatusCode::CREATED, Json(item)).into_response())
    }

    /// Handle PUT /api/notifications/scheduled/:id - Update a scheduled notification
    async fn handle_update_scheduled(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
        Path(id): Path<Uuid>,
        Json(request): Json<UpdateScheduledNotificationRequest>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        // Validate cron expression if provided
        if let Some(ref cron) = request.schedule_cron {
            validate_cron_expression(cron).map_err(AppError::invalid_input)?;
        }

        // Validate timezone if provided
        if let Some(ref tz) = request.timezone {
            tz.parse::<chrono_tz::Tz>()
                .map_err(|_| AppError::invalid_input(format!("Invalid timezone: {tz}")))?;
        }

        let service = Self::get_service(&resources)?;

        // Recompute next fire time if cron or timezone changed
        let next_fire_at = if request.schedule_cron.is_some() || request.timezone.is_some() {
            // Fetch the specific schedule by ID to get current values for unchanged fields
            let current = service
                .get_scheduled_notification_by_id(id, auth.user_id, tenant_id)
                .await?
                .ok_or_else(|| AppError::not_found("Scheduled notification"))?;

            let cron = request
                .schedule_cron
                .as_deref()
                .unwrap_or(&current.schedule_cron);
            let tz = request.timezone.as_deref().unwrap_or(&current.timezone);

            compute_next_fire_time(cron, tz, Utc::now())
        } else {
            None
        };

        let update_params = UpdateScheduledNotificationParams {
            id,
            user_id: auth.user_id,
            tenant_id,
            enabled: request.enabled,
            schedule_cron: request.schedule_cron,
            timezone: request.timezone,
            next_fire_at,
        };

        let updated = service
            .update_scheduled_notification(&update_params)
            .await?;

        if !updated {
            return Err(AppError::not_found("Scheduled notification"));
        }

        Ok((StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response())
    }

    /// Handle DELETE /api/notifications/scheduled/:id - Delete a scheduled notification
    async fn handle_delete_scheduled(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
        Path(id): Path<Uuid>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        let service = Self::get_service(&resources)?;
        let deleted = service
            .delete_scheduled_notification(id, auth.user_id, tenant_id)
            .await?;

        if !deleted {
            return Err(AppError::not_found("Scheduled notification"));
        }

        Ok((StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response())
    }

    /// Handle POST /api/notifications/badge-sync - Get unread count for badge display
    async fn handle_badge_sync(
        State(resources): State<Arc<ServerResources>>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();
        let tenant_id = Self::get_tenant_id(&auth)?;

        let service = Self::get_service(&resources)?;
        let count = service.get_unread_count(auth.user_id, tenant_id).await?;

        Ok((StatusCode::OK, Json(serde_json::json!({"count": count}))).into_response())
    }
}
