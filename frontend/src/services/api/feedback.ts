// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The feedback thread read and write — what the athlete sends the Dravr team, read back oldest first (carnet#1235)
// ABOUTME: Each answer is checked with its shared-types parser, so a malformed body is an error rather than a guessed thread

import {
  parseFeedbackCreated,
  parseFeedbackThread,
  type FeedbackMessage,
  type FeedbackThreadResponse,
  type SendFeedbackRequest,
} from '@pierre/shared-types';
import { axios } from './client';

const FEEDBACK_THREAD = '/api/me/feedback';

/**
 * The athlete's thread with the Dravr team. Nothing answers it
 * automatically: the team reads each message where the server reports it.
 */
// LIMITATION(registre#1241): feedbackApi is web-only — the phone has no feedback thread to call it.
export const feedbackApi = {
  /** The newest messages of the athlete's thread, oldest first. */
  async getThread(): Promise<FeedbackThreadResponse> {
    const response = await axios.get<unknown>(FEEDBACK_THREAD);
    const parsed = parseFeedbackThread(response.data);
    if (parsed === null) {
      throw new Error(`Unexpected response shape from ${FEEDBACK_THREAD}`);
    }
    return parsed;
  },

  /**
   * Send one message. Rejects with 400 for an empty message or one past
   * `FEEDBACK_MESSAGE_MAX_CHARS`, and with 404 when `about` names a reply
   * the athlete did not rate.
   */
  async sendMessage(request: SendFeedbackRequest): Promise<FeedbackMessage> {
    const response = await axios.post<unknown>(FEEDBACK_THREAD, request);
    const parsed = parseFeedbackCreated(response.data);
    if (parsed === null) {
      throw new Error(`Unexpected response shape from ${FEEDBACK_THREAD}`);
    }
    return parsed;
  },
};
