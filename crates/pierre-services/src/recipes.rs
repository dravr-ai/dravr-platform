// ABOUTME: Agent version diff computation extracted from the version-history route handlers
// ABOUTME: Compares two version snapshots field by field
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// A field-level change between two agent version snapshots
#[derive(Debug)]
pub struct FieldChange {
    /// Name of the changed field
    pub field: String,
    /// Previous value (None if field was added)
    pub old_value: Option<serde_json::Value>,
    /// New value (None if field was removed)
    pub new_value: Option<serde_json::Value>,
}

/// Compute field-level differences between two JSON agent version snapshots
///
/// Compares specific agent fields (title, description, `system_prompt`, category,
/// tags, `sample_prompts`, visibility) and returns a list of changes.
#[must_use]
pub fn compute_version_diff(from: &serde_json::Value, to: &serde_json::Value) -> Vec<FieldChange> {
    let mut changes = Vec::new();

    let fields = [
        "title",
        "description",
        "system_prompt",
        "category",
        "tags",
        "sample_prompts",
        "visibility",
    ];

    for field in fields {
        let old_val = from.get(field);
        let new_val = to.get(field);

        match (old_val, new_val) {
            (Some(old), Some(new)) if old != new => {
                changes.push(FieldChange {
                    field: field.to_owned(),
                    old_value: Some(old.clone()),
                    new_value: Some(new.clone()),
                });
            }
            (None, Some(new)) => {
                changes.push(FieldChange {
                    field: field.to_owned(),
                    old_value: None,
                    new_value: Some(new.clone()),
                });
            }
            (Some(old), None) => {
                changes.push(FieldChange {
                    field: field.to_owned(),
                    old_value: Some(old.clone()),
                    new_value: None,
                });
            }
            _ => {}
        }
    }

    changes
}
