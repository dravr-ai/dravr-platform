// ABOUTME: Universal protocol module with clean architecture
// ABOUTME: Modular components for tool execution, authentication, and routing

pub mod auth_service;
pub mod executor;
pub mod handlers;
pub mod tool_registry;

// Re-export main types from the old universal.rs for backward compatibility
pub use crate::protocols::universal_old::{
    UniversalRequest, UniversalResponse, UniversalTool, UniversalToolExecutor,
};

// Re-export new architecture components
pub use auth_service::AuthService;
pub use executor::UniversalExecutor;
pub use tool_registry::ToolRegistry;
