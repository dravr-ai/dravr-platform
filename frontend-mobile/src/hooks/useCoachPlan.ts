// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile client's binding of the shared plan-thread hooks — Group info's "Work on" and "View" plan actions, and the draft card
// ABOUTME: The hooks themselves live in @pierre/ui-logic, one definition for web and mobile

import { createCoachPlanHooks } from '@pierre/ui-logic';
import { chatApi } from '../services/api';

/**
 * The plan-thread hooks over the mobile chat API. The API is reached when a
 * hook calls it, so importing this touches no client.
 */
export const { useOpenAthleteThread, useAthletePlan, usePlanDraftCard } = createCoachPlanHooks({
  chatApi: {
    openAthleteThread: (athleteUserId, groupId) => chatApi.openAthleteThread(athleteUserId, groupId),
    getAthletePlan: (athleteUserId, groupId) => chatApi.getAthletePlan(athleteUserId, groupId),
    getSubjectPlan: (conversationId) => chatApi.getSubjectPlan(conversationId),
    publishSubjectPlan: (conversationId, request) => chatApi.publishSubjectPlan(conversationId, request),
    discardSubjectPlanDraft: (conversationId) => chatApi.discardSubjectPlanDraft(conversationId),
    listParticipants: (conversationId) => chatApi.listParticipants(conversationId),
  },
});
