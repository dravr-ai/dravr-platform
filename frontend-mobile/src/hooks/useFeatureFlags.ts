// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile client's binding of the shared feature-flag hook to its feature flags API
// ABOUTME: Types, keys and the failure answer all come from the shared @pierre/api-client domain

import { createFeatureFlagsHook } from '@pierre/ui-logic';
import { featureFlagsApi } from '../services/api';

export { FEATURE_KEYS } from '@pierre/api-client';

export const useFeatureFlags = createFeatureFlagsHook(featureFlagsApi);
