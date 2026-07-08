// ABOUTME: Re-export shim — the onboarding step registry lives in @pierre/shared-constants (shared web + mobile)
// ABOUTME: Kept so web imports of '../onboarding/steps' resolve unchanged; the single source is the shared package

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

export * from '@pierre/shared-constants/onboarding';
