// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The group invite a /groups/join/<code> link carried: the server holds it, then joins it once the account is active
// ABOUTME: Web-only: that link opens the web app, while the phone's own invite deep link joins by /group join (carnet#1648)

import type { HoldGroupInviteResponse, JoinHeldInviteResponse } from '@pierre/shared-types';
import { axios } from './client';

/** Hold the carried invite against the signed-in account, pending or active. */
const GROUP_INVITE_HOLD = '/api/user/group-invite';
/** Join the group the server holds an invite to for the caller, and drop the hold. */
const JOIN_HELD_INVITE = '/api/chat/groups/join';

export const groupInviteApi = {
  /**
   * Have the server hold the invite a `/groups/join/<code>` link carried. A
   * 400 or 404 means the code is dead and nothing is worth retrying; a 429, a
   * 5xx or no answer leaves it to try again on a later page load. The answer
   * says which kind of invite it is: a member's, or the group's coach seat.
   */
  async holdGroupInvite(code: string): Promise<HoldGroupInviteResponse> {
    const response = await axios.put<HoldGroupInviteResponse>(GROUP_INVITE_HOLD, { code });
    return response.data;
  },

  /**
   * Join the group the server holds an invite to for the caller (an active
   * account). Every decided outcome answers 200, nothing held included; a
   * failed call keeps the hold.
   */
  async joinHeldInvite(): Promise<JoinHeldInviteResponse> {
    const response = await axios.post<JoinHeldInviteResponse>(JOIN_HELD_INVITE);
    return response.data;
  },
};
