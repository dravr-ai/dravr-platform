// ABOUTME: MCP tool schema definitions for coach management and admin coach tools
// ABOUTME: Covers user coach CRUD, activation, favorites, hiding, and admin system coach operations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use crate::constants::{
    json_fields::{LIMIT, OFFSET},
    tools::{
        ACTIVATE_COACH, ADMIN_ASSIGN_COACH, ADMIN_CREATE_SYSTEM_COACH, ADMIN_DELETE_SYSTEM_COACH,
        ADMIN_GET_SYSTEM_COACH, ADMIN_LIST_COACH_ASSIGNMENTS, ADMIN_LIST_SYSTEM_COACHES,
        ADMIN_UNASSIGN_COACH, ADMIN_UPDATE_SYSTEM_COACH, CREATE_COACH, DEACTIVATE_COACH,
        DELETE_COACH, GET_ACTIVE_COACH, GET_COACH, HIDE_COACH, LIST_COACHES, LIST_HIDDEN_COACHES,
        SEARCH_COACHES, SHOW_COACH, TOGGLE_COACH_FAVORITE, UPDATE_COACH,
    },
};

use super::{JsonSchema, PropertySchema, ToolSchema};

/// Create coach management and admin coach tool schemas
pub(super) fn create_coaches_tools() -> Vec<ToolSchema> {
    vec![
        // Coach Management Tools (custom AI personas)
        create_list_coaches_tool(),
        create_create_coach_tool(),
        create_get_coach_tool(),
        create_update_coach_tool(),
        create_delete_coach_tool(),
        create_toggle_coach_favorite_tool(),
        create_search_coaches_tool(),
        create_activate_coach_tool(),
        create_deactivate_coach_tool(),
        create_get_active_coach_tool(),
        create_hide_coach_tool(),
        create_show_coach_tool(),
        create_list_hidden_coaches_tool(),
        // Admin Coach Management Tools (system coaches - admin only)
        create_admin_list_system_coaches_tool(),
        create_admin_create_system_coach_tool(),
        create_admin_get_system_coach_tool(),
        create_admin_update_system_coach_tool(),
        create_admin_delete_system_coach_tool(),
        create_admin_assign_coach_tool(),
        create_admin_unassign_coach_tool(),
        create_admin_list_coach_assignments_tool(),
    ]
}

/// Create the `list_coaches` tool schema
fn create_list_coaches_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "category".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Filter by category: 'training', 'nutrition', 'recovery', 'recipes', 'custom' (optional)".into()),
        },
    );

    properties.insert(
        "favorites_only".to_owned(),
        PropertySchema {
            property_type: "boolean".into(),
            description: Some("If true, only return favorite coaches (default: false)".into()),
        },
    );

    properties.insert(
        LIMIT.to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Maximum number of coaches to return (default: 50)".into()),
        },
    );

    properties.insert(
        OFFSET.to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Number of coaches to skip for pagination (default: 0)".into()),
        },
    );

    properties.insert(
        "include_system".to_owned(),
        PropertySchema {
            property_type: "boolean".into(),
            description: Some("Include system coaches created by admins (default: true)".into()),
        },
    );

    properties.insert(
        "include_hidden".to_owned(),
        PropertySchema {
            property_type: "boolean".into(),
            description: Some("Include coaches the user has hidden (default: false)".into()),
        },
    );

    ToolSchema {
        name: LIST_COACHES.to_owned(),
        description:
            "List AI coaches including personal, system (admin-created), and assigned coaches. Returns is_system and is_assigned flags for each coach.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: None,
        },
        annotations: None,
    }
}

/// Create the `create_coach` tool schema
fn create_create_coach_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "title".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Name of the coach (e.g., 'Marathon Training Coach')".into()),
        },
    );

    properties.insert(
        "system_prompt".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "The system prompt that defines the coach's persona and expertise".into(),
            ),
        },
    );

    properties.insert(
        "description".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Brief description of what this coach specializes in (optional)".into(),
            ),
        },
    );

    properties.insert(
        "category".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Category: 'training', 'nutrition', 'recovery', 'recipes', 'custom' (default: 'custom')".into()),
        },
    );

    properties.insert(
        "tags".to_owned(),
        PropertySchema {
            property_type: "array".into(),
            description: Some(
                "Tags for searchability (e.g., ['running', 'marathon', 'endurance'])".into(),
            ),
        },
    );

    ToolSchema {
        name: CREATE_COACH.to_owned(),
        description: "Create a new custom AI coach with a specific persona and system prompt."
            .into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["title".to_owned(), "system_prompt".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `get_coach` tool schema
fn create_get_coach_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "coach_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the coach to retrieve".into()),
        },
    );

    ToolSchema {
        name: GET_COACH.to_owned(),
        description: "Get details of a specific coach by ID.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `update_coach` tool schema
fn create_update_coach_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "coach_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the coach to update".into()),
        },
    );

    properties.insert(
        "title".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("New name for the coach (optional)".into()),
        },
    );

    properties.insert(
        "system_prompt".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("New system prompt (optional)".into()),
        },
    );

    properties.insert(
        "description".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("New description (optional)".into()),
        },
    );

    properties.insert(
        "category".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("New category (optional)".into()),
        },
    );

    properties.insert(
        "tags".to_owned(),
        PropertySchema {
            property_type: "array".into(),
            description: Some("New tags (optional)".into()),
        },
    );

    ToolSchema {
        name: UPDATE_COACH.to_owned(),
        description: "Update an existing coach's properties.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `delete_coach` tool schema
fn create_delete_coach_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "coach_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the coach to delete".into()),
        },
    );

    ToolSchema {
        name: DELETE_COACH.to_owned(),
        description: "Delete a coach from user's collection. This action cannot be undone.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `toggle_coach_favorite` tool schema
fn create_toggle_coach_favorite_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "coach_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the coach to toggle favorite status".into()),
        },
    );

    ToolSchema {
        name: TOGGLE_COACH_FAVORITE.to_owned(),
        description: "Toggle a coach's favorite status. Returns the updated coach.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `search_coaches` tool schema
fn create_search_coaches_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "query".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Search query for coach title, description, or tags".into()),
        },
    );

    properties.insert(
        LIMIT.to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Maximum number of results to return (default: 20)".into()),
        },
    );

    ToolSchema {
        name: SEARCH_COACHES.to_owned(),
        description: "Search user's coaches by title, description, or tags.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["query".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `activate_coach` tool schema
fn create_activate_coach_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "coach_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the coach to activate".into()),
        },
    );

    ToolSchema {
        name: ACTIVATE_COACH.to_owned(),
        description: "Activate a coach for the current session. Only one coach can be active at a time; activating a new coach deactivates any previously active coach.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `deactivate_coach` tool schema
fn create_deactivate_coach_tool() -> ToolSchema {
    ToolSchema {
        name: DEACTIVATE_COACH.to_owned(),
        description: "Deactivate the currently active coach.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: None,
            required: None,
        },
        annotations: None,
    }
}

/// Create the `get_active_coach` tool schema
fn create_get_active_coach_tool() -> ToolSchema {
    ToolSchema {
        name: GET_ACTIVE_COACH.to_owned(),
        description: "Get the currently active coach for the user, if any.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: None,
            required: None,
        },
        annotations: None,
    }
}

/// Create the `hide_coach` tool schema
fn create_hide_coach_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "coach_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the coach to hide".into()),
        },
    );

    ToolSchema {
        name: HIDE_COACH.to_owned(),
        description: "Hide a system or assigned coach from the user's coach list. Only system coaches or assigned coaches can be hidden - personal coaches cannot be hidden.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `show_coach` tool schema
fn create_show_coach_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "coach_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the coach to show (unhide)".into()),
        },
    );

    ToolSchema {
        name: SHOW_COACH.to_owned(),
        description: "Show (unhide) a previously hidden coach.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `list_hidden_coaches` tool schema
fn create_list_hidden_coaches_tool() -> ToolSchema {
    ToolSchema {
        name: LIST_HIDDEN_COACHES.to_owned(),
        description: "List all coaches that the user has hidden.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: None,
            required: None,
        },
        annotations: None,
    }
}

// ============================================================================
// Admin Coach Management Tools (System Coaches - Admin Only)
// ============================================================================

/// Create the `admin_list_system_coaches` tool schema
fn create_admin_list_system_coaches_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "visibility".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Filter by visibility level: 'tenant' or 'global' (optional)".into()),
        },
    );

    properties.insert(
        LIMIT.to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Maximum number of coaches to return (default: 50)".into()),
        },
    );

    properties.insert(
        OFFSET.to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Number of coaches to skip for pagination (default: 0)".into()),
        },
    );

    ToolSchema {
        name: ADMIN_LIST_SYSTEM_COACHES.to_owned(),
        description:
            "List system coaches for the tenant. Admin only. Returns coaches with is_system=true."
                .into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: None,
        },
        annotations: None,
    }
}

/// Create the `admin_create_system_coach` tool schema
fn create_admin_create_system_coach_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "title".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Name of the system coach (e.g., 'Elite Marathon Coach')".into()),
        },
    );

    properties.insert(
        "system_prompt".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "The system prompt that defines the coach's persona and expertise".into(),
            ),
        },
    );

    properties.insert(
        "description".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Brief description of what this coach specializes in (optional)".into(),
            ),
        },
    );

    properties.insert(
        "category".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Category: 'training', 'nutrition', 'recovery', 'recipes', 'custom' (default: 'custom')".into()),
        },
    );

    properties.insert(
        "tags".to_owned(),
        PropertySchema {
            property_type: "array".into(),
            description: Some(
                "Tags for searchability (e.g., ['running', 'marathon', 'elite'])".into(),
            ),
        },
    );

    properties.insert(
        "visibility".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Visibility level: 'tenant' (visible to tenant users) or 'global' (all tenants). Default: 'tenant'".into(),
            ),
        },
    );

    ToolSchema {
        name: ADMIN_CREATE_SYSTEM_COACH.to_owned(),
        description: "Create a new system coach that can be assigned to users. Admin only.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["title".to_owned(), "system_prompt".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `admin_get_system_coach` tool schema
fn create_admin_get_system_coach_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "coach_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the system coach to retrieve".into()),
        },
    );

    ToolSchema {
        name: ADMIN_GET_SYSTEM_COACH.to_owned(),
        description: "Get details of a specific system coach by ID. Admin only.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `admin_update_system_coach` tool schema
fn create_admin_update_system_coach_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "coach_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the system coach to update".into()),
        },
    );

    properties.insert(
        "title".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("New name for the coach (optional)".into()),
        },
    );

    properties.insert(
        "system_prompt".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("New system prompt for the coach (optional)".into()),
        },
    );

    properties.insert(
        "description".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("New description for the coach (optional)".into()),
        },
    );

    properties.insert(
        "category".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("New category for the coach (optional)".into()),
        },
    );

    properties.insert(
        "tags".to_owned(),
        PropertySchema {
            property_type: "array".into(),
            description: Some("New tags for the coach (optional)".into()),
        },
    );

    properties.insert(
        "visibility".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("New visibility level: 'tenant' or 'global' (optional)".into()),
        },
    );

    ToolSchema {
        name: ADMIN_UPDATE_SYSTEM_COACH.to_owned(),
        description: "Update an existing system coach. Admin only.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `admin_delete_system_coach` tool schema
fn create_admin_delete_system_coach_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "coach_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the system coach to delete".into()),
        },
    );

    ToolSchema {
        name: ADMIN_DELETE_SYSTEM_COACH.to_owned(),
        description: "Delete a system coach. This will also remove all assignments. Admin only."
            .into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `admin_assign_coach` tool schema
fn create_admin_assign_coach_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "coach_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the system coach to assign".into()),
        },
    );

    properties.insert(
        "user_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the user to assign the coach to".into()),
        },
    );

    ToolSchema {
        name: ADMIN_ASSIGN_COACH.to_owned(),
        description: "Assign a system coach to a specific user. Admin only.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned(), "user_id".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `admin_unassign_coach` tool schema
fn create_admin_unassign_coach_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "coach_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the system coach to unassign".into()),
        },
    );

    properties.insert(
        "user_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the user to unassign the coach from".into()),
        },
    );

    ToolSchema {
        name: ADMIN_UNASSIGN_COACH.to_owned(),
        description: "Remove a system coach assignment from a user. Admin only.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned(), "user_id".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `admin_list_coach_assignments` tool schema
fn create_admin_list_coach_assignments_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "coach_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Filter by coach ID (optional)".into()),
        },
    );

    properties.insert(
        "user_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Filter by user ID (optional)".into()),
        },
    );

    properties.insert(
        LIMIT.to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Maximum number of assignments to return (default: 100)".into()),
        },
    );

    properties.insert(
        OFFSET.to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Number of assignments to skip for pagination (default: 0)".into()),
        },
    );

    ToolSchema {
        name: ADMIN_LIST_COACH_ASSIGNMENTS.to_owned(),
        description: "List coach assignments in the tenant with optional filtering. Admin only."
            .into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: None,
        },
        annotations: None,
    }
}
