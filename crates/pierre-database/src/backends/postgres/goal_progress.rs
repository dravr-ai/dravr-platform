// ABOUTME: Derives the stored goal JSON's progress fields when a goal's current value changes
// ABOUTME: Shared shape with the SQLite backend: current_value, last_updated, and a clamped progress_percentage
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
use pierre_core::errors::{AppError, AppResult};
use serde_json::Value;

/// Write `current_value`, `last_updated`, and — when the goal carries a
/// positive `target_value` — a clamped `progress_percentage` into the goal
/// JSON, the same derived fields the `SQLite` backend computes on update.
pub(super) fn apply_progress_fields(goal_data: &mut Value, current_value: f64) -> AppResult<()> {
    let Some(obj) = goal_data.as_object_mut() else {
        return Ok(());
    };
    obj.insert(
        "current_value".into(),
        Value::Number(serde_json::Number::from_f64(current_value).ok_or_else(|| {
            AppError::internal(format!("Invalid current_value: {current_value}"))
        })?),
    );
    obj.insert(
        "last_updated".into(),
        Value::String(chrono::Utc::now().to_rfc3339()),
    );
    if let Some(target) = obj.get("target_value").and_then(Value::as_f64) {
        if target > 0.0 {
            let progress_percentage = (current_value / target * 100.0).clamp(0.0, 100.0);
            obj.insert(
                "progress_percentage".into(),
                Value::Number(
                    serde_json::Number::from_f64(progress_percentage).ok_or_else(|| {
                        AppError::internal(format!(
                            "Invalid progress_percentage: {progress_percentage}"
                        ))
                    })?,
                ),
            );
        }
    }
    Ok(())
}
