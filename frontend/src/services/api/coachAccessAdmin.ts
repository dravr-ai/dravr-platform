// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Admin console client for the coach-access queue — list requests, grant or decline one (carnet#738)
// ABOUTME: Super-admin only on the server; same session-authenticated axios and AdminResponse envelope as admin.ts

import type { CoachAccessRequest, CoachAccessRequestView, CoachAccessStatus } from '@pierre/shared-types';
import { axios } from './client';

/** What a grant did: the request, and the groups it made the coach coach. */
export interface CoachAccessGrantResult {
  message: string;
  request: CoachAccessRequest;
  /** The first group the coach now coaches (the named one first); `null` when none was attached. */
  attachedGroupId: string | null;
  /** Every coachless group the coach owned, now coached by them (carnet#1521). */
  attachedGroupIds: string[];
}

/** What a decline did. */
export interface CoachAccessDeclineResult {
  message: string;
  request: CoachAccessRequest;
}

export const coachAccessAdminApi = {
  /** The requests in `status` (pending by default), oldest first. */
  async list(status: CoachAccessStatus = 'pending'): Promise<CoachAccessRequestView[]> {
    const response = await axios.get('/api/admin/coach-access-requests', { params: { status } });
    // A body without the list is a failed read, never an empty queue: the
    // console would otherwise say no coach is waiting while one is.
    const requests: unknown = response.data?.data?.requests;
    if (!Array.isArray(requests)) {
      throw new Error('Malformed coach access request listing');
    }
    return requests as CoachAccessRequestView[];
  },

  /** Grant coach access, attaching the coach to the group they asked from. */
  async grant(requestId: string): Promise<CoachAccessGrantResult> {
    const response = await axios.post(
      `/api/admin/coach-access-requests/${encodeURIComponent(requestId)}/grant`,
    );
    return {
      message: response.data.message,
      request: response.data.data.request,
      attachedGroupId: response.data.data.attached_group_id ?? null,
      attachedGroupIds: response.data.data.attached_group_ids ?? [],
    };
  },

  /**
   * Decline a request; the coach may ask again. `reason`, when given, is shown
   * to the coach in the notice, the email and their declined state.
   */
  async decline(requestId: string, reason?: string): Promise<CoachAccessDeclineResult> {
    const trimmed = reason?.trim();
    const response = await axios.post(
      `/api/admin/coach-access-requests/${encodeURIComponent(requestId)}/decline`,
      trimmed ? { reason: trimmed } : {},
    );
    return { message: response.data.message, request: response.data.data.request };
  },
};
