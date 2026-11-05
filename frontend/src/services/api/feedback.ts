// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The feedback thread read and write, and its unread team replies — the athlete and the Dravr team (carnet#1235, #1392)
// ABOUTME: Each answer is checked with its shared-types parser, so a malformed body is an error rather than a guessed thread

import {
  parseFeedbackCreated,
  parseFeedbackThread,
  parseFeedbackUnread,
  type FeedbackMessage,
  type FeedbackThreadResponse,
  type SendFeedbackRequest,
} from '@pierre/shared-types';
import { axios } from './client';

const FEEDBACK_THREAD = '/api/me/feedback';
const FEEDBACK_UNREAD = '/api/me/feedback/unread';
const FEEDBACK_READ = '/api/me/feedback/read';

/**
 * The athlete's thread with the Dravr team. Nothing answers it
 * automatically: the team reads each message where the server reports it
 * and replies from there; a reply waits unread until the thread is opened.
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

  /** How many of the team's replies the athlete has not opened yet. */
  async getUnread(): Promise<number> {
    const response = await axios.get<unknown>(FEEDBACK_UNREAD);
    const parsed = parseFeedbackUnread(response.data);
    if (parsed === null) {
      throw new Error(`Unexpected response shape from ${FEEDBACK_UNREAD}`);
    }
    return parsed.unread;
  },

  /** The screenshot one of the athlete's own messages carries. */
  async getImage(messageId: string): Promise<Blob> {
    const path = `/api/me/feedback/${encodeURIComponent(messageId)}/image`;
    const response = await axios.get<unknown>(path, { responseType: 'blob' });
    if (!(response.data instanceof Blob)) {
      throw new Error(`Unexpected response shape from ${path}`);
    }
    return response.data;
  },

  /** The athlete has the thread open: every team reply in it is read. */
  async markRead(): Promise<void> {
    await axios.post<unknown>(FEEDBACK_READ);
  },
};
