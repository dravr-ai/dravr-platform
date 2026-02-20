// ABOUTME: Re-exports coach types from pierre-core for internal module paths
// ABOUTME: All type definitions now live in pierre-core/src/models/coaches.rs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

pub use pierre_core::models::coaches::{
    Coach, CoachAssignment, CoachCategory, CoachListItem, CoachVersion, CoachVisibility,
    CreateCoachRequest, CreateSystemCoachRequest, ListCoachesFilter, PublishStatus,
    StoreAdminStats, UpdateCoachRequest,
};
