// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Binds the shared physiology read to this client's athlete API
// ABOUTME: The athlete's anchors per sport family, which Settings › Physiology lists

import { useTranslation } from '@pierre/i18n';
import { createPhysiologyHook } from '@pierre/ui-logic';
import { athleteApi } from '../services/api';

/**
 * The definition lives in `@pierre/ui-logic`, one for web and mobile; the
 * API is reached when the read runs, so importing this touches no client.
 */
export const usePhysiology = createPhysiologyHook(
  { getPhysiology: (locale) => athleteApi.getPhysiology(locale) },
  { useLanguage: () => useTranslation().language },
);
