// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web binding of the coach-access hooks — a grant picked up without signing out, and the Groups list's coach-seat line
// ABOUTME: The definitions live in @pierre/ui-logic, one for web and phone; this hands in the web's APIs, session and words

import { createCoachAccessHooks } from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';
import { authApi, groupsApi, userApi } from '../services/api';
import { useAuth } from './useAuth';

/**
 * The coach-access hooks over the web client. The APIs are reached when a
 * hook runs, so importing this touches no client. The web reads the waiting
 * request again when the window regains focus.
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
  refetchOnWindowFocus: true,
});
