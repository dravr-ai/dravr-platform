// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Address confirmation from a mailed link — confirm on a press, or re-send from a link that no longer verifies
// ABOUTME: Web-only: the mailed confirmation link opens the web app's confirmation page, never the mobile app

import { axios } from './client';
import type { EmailConfirmationResponse } from '@pierre/shared-types';

export const emailConfirmationApi = {
  /**
   * Confirm an address with the token from its mailed link.
   *
   * Call it only from a person's press of the confirmation page's button:
   * mail scanners load every link, and some run the page's script, so a
   * confirmation fired on load would spend the token before its owner saw it.
   */
  async confirmEmail(token: string): Promise<EmailConfirmationResponse> {
    const response = await axios.post<EmailConfirmationResponse>('/api/auth/verify-email', {
      token,
    });
    return response.data;
  },

  /**
   * Re-send a confirmation link from a link that no longer verifies, so the
   * person is never asked to type the address the link was sent to. Answers
   * with the same neutral message as a resend by address.
   */
  async resendForLink(token: string): Promise<{ message: string }> {
    const response = await axios.post<{ message: string }>('/api/auth/resend-verification', {
      token,
    });
    return response.data;
  },
};
