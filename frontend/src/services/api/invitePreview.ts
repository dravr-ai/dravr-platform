// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The invite card's public read of a group invite code — who sent it, to which group, and whether it works (carnet#1649)
// ABOUTME: Web-only: /groups/join/<code> opens the web app's card; the phone's link holds the invite and Home names it

import type { GroupInvitePreview } from '@pierre/shared-types';
import { axios } from './client';

export const invitePreviewApi = {
  /**
   * The card for `code`. Needs no session; a session belonging to the link's
   * coach is not counted as an opening. 404 for an unknown code (the same for
   * every code), 429 once this client has asked too often.
   */
  async previewInvite(code: string): Promise<GroupInvitePreview> {
    const response = await axios.get<GroupInvitePreview>(`/api/invites/${encodeURIComponent(code)}`);
    return response.data;
  },
};
