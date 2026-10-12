// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Invitation acceptance — the invitee chooses a password and the account is created active
// ABOUTME: Web-only: the mailed invitation link opens the web app's accept page, never the mobile app

import { axios } from './client';
import type { RegisterResponse } from '@pierre/shared-types';

export const invitationsApi = {
  /**
   * Accept an invitation with the token from its link and the chosen password.
   *
   * The token is single-use: a second call with it answers 404, as does an
   * expired or revoked one, and the server never says which.
   */
  async acceptInvitation(
    token: string,
    password: string,
    displayName?: string,
  ): Promise<RegisterResponse> {
    const response = await axios.post<RegisterResponse>('/api/auth/accept-invitation', {
      token,
      password,
      display_name: displayName || null,
    });
    return response.data;
  },
};
