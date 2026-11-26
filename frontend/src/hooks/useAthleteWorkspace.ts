// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web binding of the athlete workspace hooks — the workspace's two threads and the coach's answers to the draft
// ABOUTME: The definitions live in @pierre/ui-logic, one for web and phone; this hands in the web's APIs, list, plan and day-editor hooks, words and toasts

import { createWorkspaceHooks } from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';
import { groupsApi } from '../services/api';
import { useErrorToast } from '../components/ui/useToast';
import { useAuth } from './useAuth';
import { useConversationList } from './useConversationList';
import { useOpenAthleteThread, useOpenGroupRoom } from './useCoachPlan';
import { WEB_GROUP_FRESHNESS } from './useGroups';
import { useRereadPlan, useRevertDraftDate, useSetCoachCall } from './useSessionEditor';

/**
 * The workspace hooks over the web client. The APIs are reached when a hook
 * runs, so importing this touches no client. A refused write says why in a
 * toast.
 */
export const { useWorkspaceThreads, useWorkspaceAnswers } = createWorkspaceHooks({
  groupsApi: { getGroup: (groupId) => groupsApi.getGroup(groupId) },
  groupStaleMs: WEB_GROUP_FRESHNESS.detail,
  useViewerId: () => {
    const { user } = useAuth();
    return user?.id ?? null;
  },
  useConversations: () => {
    const { conversations, isLoading } = useConversationList();
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
  useReportError: () => {
    const showError = useErrorToast();
    return (title, detail) => showError(title, detail);
  },
});
