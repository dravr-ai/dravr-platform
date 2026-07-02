// ABOUTME: Unit tests for the pluggable tools framework (Phase 1 Foundation)
// ABOUTME: Tests traits, registry, context, result, errors, and decorator implementations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::ToolCapabilities;
use pierre_tools_core::{NotificationType, ToolError, ToolNotification, ToolResult};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

use dravr_tronc::mcp::schema::{Content, Tool, ToolResponse};
use dravr_tronc::mcp::tool::{
    McpTool, ToolCapabilities as TroncCapabilities, ToolContext as TroncToolContext,
};

// A simple stub tool for testing. The tronc trait keys discovery and gating off
// the host-agnostic capability set, so the stub stores tronc capabilities.
struct StubTool {
    name: &'static str,
    capabilities: TroncCapabilities,
}

impl StubTool {
    const fn new(name: &'static str, capabilities: TroncCapabilities) -> Self {
        Self { name, capabilities }
    }
}

#[async_trait]
impl McpTool<dyn ToolRuntime> for StubTool {
    fn definition(&self) -> Tool {
        Tool {
            name: self.name.to_owned(),
            description: "Stub tool for unit testing".to_owned(),
            input_schema: serde_json::json!({"type": "object"}),
            annotations: None,
        }
    }

    fn capabilities(&self) -> TroncCapabilities {
        self.capabilities
    }

    async fn execute(
        &self,
        _state: &Arc<dyn ToolRuntime>,
        _ctx: &TroncToolContext,
        _args: Value,
    ) -> ToolResponse {
        ToolResponse {
            content: vec![Content::Text {
                text: serde_json::json!({"status": "ok"}).to_string(),
            }],
            is_error: false,
            structured_content: Some(serde_json::json!({"status": "ok"})),
        }
    }
}

// ============================================================================
// ToolCapabilities Tests
// ============================================================================

mod capabilities_tests {
    use super::*;

    #[test]
    fn test_capabilities_empty() {
        let caps = ToolCapabilities::empty();
        assert!(caps.is_empty());
    }

    #[test]
    fn test_capabilities_combination() {
        let caps = ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA;
        assert!(caps.requires_auth());
        assert!(caps.reads_data());
        assert!(!caps.writes_data());
        assert!(!caps.is_admin_only());
    }

    #[test]
    fn test_capabilities_admin_only() {
        let caps = ToolCapabilities::ADMIN_ONLY | ToolCapabilities::REQUIRES_AUTH;
        assert!(caps.is_admin_only());
        assert!(caps.requires_auth());
    }

    #[test]
    fn test_capabilities_describe() {
        let caps = ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA;
        let desc = caps.describe();
        assert!(desc.contains("requires_auth"));
        assert!(desc.contains("reads_data"));
    }
}

// ============================================================================
// ToolRegistry Tests
// ============================================================================

mod registry_tests {
    use super::*;

    #[test]
    fn test_registry_register() {
        let mut registry = ToolRegistry::new();
        let tool = Arc::new(StubTool::new("test_tool", TroncCapabilities::REQUIRES_AUTH));

        assert!(registry.register(tool));
        assert!(registry.contains("test_tool"));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_registry_duplicate_registration() {
        let mut registry = ToolRegistry::new();
        let tool1 = Arc::new(StubTool::new("test_tool", TroncCapabilities::REQUIRES_AUTH));
        let tool2 = Arc::new(StubTool::new("test_tool", TroncCapabilities::READS_DATA));

        assert!(registry.register(tool1));
        assert!(!registry.register(tool2)); // Should return false for duplicate
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_registry_admin_filtering() {
        let mut registry = ToolRegistry::new();

        let user_tool = Arc::new(StubTool::new("user_tool", TroncCapabilities::REQUIRES_AUTH));
        let admin_tool = Arc::new(StubTool::new(
            "admin_tool",
            TroncCapabilities::REQUIRES_AUTH | TroncCapabilities::ADMIN_ONLY,
        ));

        registry.register(user_tool);
        registry.register(admin_tool);

        // Non-admin sees only user tools
        let user_schemas = registry.list_schemas_for_role(false);
        assert_eq!(user_schemas.len(), 1);
        assert_eq!(user_schemas[0].name, "user_tool");

        // Admin sees all tools
        let admin_schemas = registry.list_schemas_for_role(true);
        assert_eq!(admin_schemas.len(), 2);
    }

    #[test]
    fn test_registry_categories() {
        let mut registry = ToolRegistry::new();

        let data_tool = Arc::new(StubTool::new("get_data", TroncCapabilities::READS_DATA));
        let analytics_tool = Arc::new(StubTool::new("analyze", TroncCapabilities::READS_DATA));

        registry.register_with_category(data_tool, "data");
        registry.register_with_category(analytics_tool, "analytics");

        assert_eq!(registry.tools_in_category("data"), vec!["get_data"]);
        assert_eq!(registry.tools_in_category("analytics"), vec!["analyze"]);
        assert!(registry.tools_in_category("unknown").is_empty());
    }

    #[test]
    fn test_registry_capability_filtering() {
        let mut registry = ToolRegistry::new();

        let read_tool = Arc::new(StubTool::new("reader", TroncCapabilities::READS_DATA));
        let write_tool = Arc::new(StubTool::new("writer", TroncCapabilities::WRITES_DATA));

        registry.register(read_tool);
        registry.register(write_tool);

        let readers = registry.read_tools();
        assert_eq!(readers, vec!["reader"]);

        let writers = registry.write_tools();
        assert_eq!(writers, vec!["writer"]);
    }
}

// ============================================================================
// ToolResult Tests
// ============================================================================

mod result_tests {
    use super::*;

    #[test]
    fn test_tool_result_ok() {
        let result = ToolResult::ok(serde_json::json!({"status": "success"}));

        assert!(!result.is_error);
        assert!(!result.has_notifications());
        assert_eq!(result.content["status"], "success");
    }

    #[test]
    fn test_tool_result_error() {
        let result = ToolResult::error(serde_json::json!({"error": "failed"}));

        assert!(result.is_error);
        assert!(!result.has_notifications());
    }

    #[test]
    fn test_tool_result_with_notifications() {
        let notification = ToolNotification::oauth_refresh("strava");
        let result = ToolResult::ok(serde_json::json!({})).add_notification(notification);

        assert!(result.has_notifications());
        assert_eq!(result.notifications.len(), 1);
    }

    #[test]
    fn test_notification_type_method_names() {
        assert_eq!(
            NotificationType::OAuthRefresh.method_name(),
            "notifications/oauth/refresh"
        );
        assert_eq!(
            NotificationType::Progress.method_name(),
            "notifications/progress"
        );
    }

    #[test]
    fn test_progress_notification() {
        let notification =
            ToolNotification::progress("token123", 50.0, Some(100.0), Some("Half done"));

        assert_eq!(notification.notification_type, NotificationType::Progress);
        assert_eq!(notification.data["progressToken"], "token123");
        assert_eq!(notification.data["progress"], 50.0);
        assert_eq!(notification.data["total"], 100.0);
        assert_eq!(notification.data["message"], "Half done");
    }

    #[test]
    fn test_from_serializable() -> Result<(), serde_json::Error> {
        #[derive(Serialize)]
        struct TestData {
            name: String,
            value: i32,
        }

        let data = TestData {
            name: "test".to_owned(),
            value: 42,
        };

        let result = ToolResult::from_serializable(&data)?;
        assert_eq!(result.content["name"], "test");
        assert_eq!(result.content["value"], 42);
        Ok(())
    }
}

// ============================================================================
// ToolError Tests
// ============================================================================

mod error_tests {
    use super::*;

    #[test]
    fn test_tool_error_not_found() {
        let error = ToolError::not_found("unknown_tool");
        assert_eq!(error.tool_name(), "unknown_tool");
    }

    #[test]
    fn test_tool_error_admin_required() {
        let error = ToolError::admin_required("admin_tool");
        assert_eq!(error.tool_name(), "admin_tool");
    }

    #[test]
    fn test_tool_error_display() {
        let error = ToolError::not_found("my_tool");
        let display = format!("{error}");
        assert!(display.contains("my_tool"));
    }

    #[test]
    fn test_tool_error_to_app_error() {
        let tool_error = ToolError::not_found("test_tool");
        let app_error: AppError = tool_error.into();
        let display = format!("{app_error}");
        assert!(display.contains("test_tool"));
    }
}

// ============================================================================
// AuthMethod Tests
// ============================================================================

mod auth_method_tests {
    use pierre_tool_runtime::context::AuthMethod;

    #[test]
    fn test_auth_method_as_str() {
        assert_eq!(AuthMethod::JwtBearer.as_str(), "jwt_bearer");
        assert_eq!(AuthMethod::ApiKey.as_str(), "api_key");
        assert_eq!(AuthMethod::OAuth2.as_str(), "oauth2");
        assert_eq!(AuthMethod::McpClient.as_str(), "mcp_client");
    }
}

// ============================================================================
// AuditedTool Decorator Tests
// ============================================================================

mod audited_tool_tests {
    use super::*;
    use pierre_tool_runtime::decorators::AuditedTool;

    struct AdminStubTool;

    #[async_trait]
    impl McpTool<dyn ToolRuntime> for AdminStubTool {
        fn definition(&self) -> Tool {
            Tool {
                name: "admin_stub_tool".to_owned(),
                description: "Test tool for unit testing".to_owned(),
                input_schema: serde_json::json!({"type": "object"}),
                annotations: None,
            }
        }

        fn capabilities(&self) -> TroncCapabilities {
            TroncCapabilities::REQUIRES_AUTH | TroncCapabilities::ADMIN_ONLY
        }

        async fn execute(
            &self,
            _state: &Arc<dyn ToolRuntime>,
            _ctx: &TroncToolContext,
            _args: Value,
        ) -> ToolResponse {
            ToolResponse::text(serde_json::json!({"status": "ok"}).to_string())
        }
    }

    pierre_tool_runtime::declare_security!(AdminStubTool => empty);

    #[test]
    fn test_audited_tool_creation() {
        let inner = Arc::new(AdminStubTool);
        let audited = AuditedTool::new(inner);

        assert_eq!(audited.definition().name, "admin_stub_tool");
        assert_eq!(
            audited.definition().description,
            "Test tool for unit testing"
        );
    }

    #[test]
    fn test_audited_tool_with_argument_logging() {
        let inner = Arc::new(AdminStubTool);
        let audited = AuditedTool::with_argument_logging(inner);

        // Just verify it can be created with argument logging enabled
        assert_eq!(audited.definition().name, "admin_stub_tool");
    }

    #[test]
    fn test_audited_tool_capabilities() {
        let inner = Arc::new(AdminStubTool);
        let audited = AuditedTool::new(inner);

        let caps = audited.capabilities();
        assert!(caps.contains(TroncCapabilities::ADMIN_ONLY));
        assert!(caps.contains(TroncCapabilities::REQUIRES_AUTH));
    }
}

pierre_tool_runtime::declare_security!(StubTool => empty);
