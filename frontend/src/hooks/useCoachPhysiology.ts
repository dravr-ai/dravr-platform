// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web client's binding of the coach's physiology hooks — a coached athlete's thresholds read and set from Group info
// ABOUTME: The hooks themselves live in @pierre/ui-logic, one definition for web and mobile (carnet#1561)

import { useTranslation } from '@pierre/i18n';
import { createCoachPhysiologyHooks } from '@pierre/ui-logic';
import { chatApi } from '../services/api';

/**
 * The coach's physiology hooks over the web chat API. The API is reached when
 * a hook calls it, so importing this touches no client.
 */
export const { useAthletePhysiology, useAthletePhysiologyEdit } = createCoachPhysiologyHooks(
  {
    getAthletePhysiology: (athleteUserId, groupId, locale) =>
      chatApi.getAthletePhysiology(athleteUserId, groupId, locale),
    updateAthletePhysiologyAnchor: (athleteUserId, groupId, sport, anchor, edit, locale) =>
      chatApi.updateAthletePhysiologyAnchor(athleteUserId, groupId, sport, anchor, edit, locale),
  },
  { useLanguage: () => useTranslation().language },
);
