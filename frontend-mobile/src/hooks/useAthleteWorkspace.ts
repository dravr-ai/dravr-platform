// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The phone binding of the athlete workspace hooks — the workspace's two threads and the coach's answers to the draft
// ABOUTME: The definitions live in @pierre/ui-logic, one for web and phone; this hands in the phone's APIs, list, plan and day-editor hooks, words and alerts

import { Alert } from 'react-native';
import { createWorkspaceHooks } from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';
import { groupsApi } from '../services/api';
import { useAuth } from '../contexts/AuthContext';
import { useConversationRows } from '../screens/conversations/useConversationList';
import { useOpenAthleteThread, useOpenGroupRoom } from './useCoachPlan';
import { MOBILE_GROUP_FRESHNESS } from './useGroups';
import { useRereadPlan, useRevertDraftDate, useSetCoachCall } from './useSessionEditor';

/** A refused write says why in an alert: its title, and the reason under it when there is one. */
function reportInAlert(title: string, detail?: string): void {
  Alert.alert(title, detail);
}

/**
 * The workspace hooks over the phone client. The APIs are reached when a
 * hook runs, so importing this touches no client.
 */
export const { useWorkspaceThreads, useWorkspaceAnswers } = createWorkspaceHooks({
  groupsApi: { getGroup: (groupId) => groupsApi.getGroup(groupId) },
  groupStaleMs: MOBILE_GROUP_FRESHNESS.detail,
  useViewerId: () => {
    const { user } = useAuth();
    return user?.id ?? null;
  },
  useConversations: () => {
    const { conversations, isLoading } = useConversationRows();
    return { conversations, isLoading };
  },
  useOpenAthleteThread,
  useOpenGroupRoom,
  useRevertDraftDate,
  useSetCoachCall,
  useRereadPlan,
  useWords: () => {
    const { t, language } = useTranslation();
    return { t, language };
  },
  useReportError: () => reportInAlert,
});
