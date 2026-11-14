// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The phone binding of the coach-access hooks — a grant picked up without signing out, and the list's coach-seat line
// ABOUTME: The definitions live in @pierre/ui-logic, one for web and phone; this hands in the phone's APIs, session and words

import { createCoachAccessHooks } from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';
import { authApi, groupsApi, userApi } from '../services/api';
import { useAuth } from '../contexts/AuthContext';

/**
 * The coach-access hooks over the phone client. The APIs are reached when a
 * hook runs, so importing this touches no client. The phone's foreground
 * read comes from its React Query focus manager.
 */
export const { useCoachAccessGrantRefresh, useGroupSeatLines } = createCoachAccessHooks({
  userApi: { getCoachAccessRequest: () => userApi.getCoachAccessRequest() },
  authApi: { getSession: () => authApi.getSession() },
  groupsApi: { getGroup: (groupId) => groupsApi.getGroup(groupId) },
  useSession: () => {
    const { user, updateUser } = useAuth();
    return { user: user ?? null, updateUser };
  },
  useWords: () => {
    const { t, language } = useTranslation();
    // i18next's own `t`, the same function every render, so the seat lines stay memoised.
    return { t, language };
  },
});
