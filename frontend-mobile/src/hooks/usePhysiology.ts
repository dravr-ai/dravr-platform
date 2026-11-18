// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Binds the shared physiology read and the athlete's own anchor save to this client's athlete API
// ABOUTME: The athlete's anchors per sport family, which Settings › Physiology lists and sets one at a time

import { useTranslation } from '@pierre/i18n';
import { createPhysiologyEditHook, createPhysiologyHook } from '@pierre/ui-logic';
import { athleteApi } from '../services/api';

const platform = { useLanguage: () => useTranslation().language };

/**
 * The definitions live in `@pierre/ui-logic`, one for web and mobile; the
 * API is reached when the read or the save runs, so importing this touches no
 * client.
 */
export const usePhysiology = createPhysiologyHook(
  { getPhysiology: (locale) => athleteApi.getPhysiology(locale) },
  platform,
);

/** The athlete sets one of their own anchors (carnet#1561). */
export const usePhysiologyEdit = createPhysiologyEditHook(
  {
    updatePhysiologyAnchor: (sport, anchor, edit, locale) =>
      athleteApi.updatePhysiologyAnchor(sport, anchor, edit, locale),
  },
  platform,
);
