// ABOUTME: A data source's device type as data_sources stores it, and back
// ABOUTME: Only the conversions live here; health_persistence.rs keeps the table's one writer (the identity upsert)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::DeviceType;

// ============================================================================
// DeviceType <-> text
// ============================================================================

/// The text `data_sources.device_type` stores for each [`DeviceType`].
pub(crate) const fn device_type_to_str(dt: DeviceType) -> &'static str {
    match dt {
        DeviceType::Watch => "watch",
        DeviceType::Band => "band",
        DeviceType::Phone => "phone",
        DeviceType::Ring => "ring",
        DeviceType::Scale => "scale",
        DeviceType::Unknown => "unknown",
    }
}

/// The inverse of [`device_type_to_str`]; anything unrecognised is `Unknown`.
pub(crate) fn str_to_device_type(s: &str) -> DeviceType {
    match s.to_lowercase().as_str() {
        "watch" => DeviceType::Watch,
        "band" => DeviceType::Band,
        "phone" => DeviceType::Phone,
        "ring" => DeviceType::Ring,
        "scale" => DeviceType::Scale,
        _ => DeviceType::Unknown,
    }
}
