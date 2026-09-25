// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web client's binding of the shared feature-flag hook to its feature flags API
// ABOUTME: Gates surfaces like the API Tokens settings tab and Billing header

import { createFeatureFlagsHook } from '@pierre/ui-logic';
import { featureFlagsApi } from '../services/api';

export { FEATURE_KEYS } from '@pierre/api-client';

export const useFeatureFlags = createFeatureFlagsHook(featureFlagsApi);
