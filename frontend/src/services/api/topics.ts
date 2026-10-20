// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The topics of a thread — one per divider, plus its opening — for the web History list
// ABOUTME: Web-only: Home's History on the web lists the personal thread's topics (carnet#840)

import { axios } from './client';

/** One topic of a thread: the stretch a divider opens, or its opening. */
export interface ThreadTopic {
  /** The divider that opens the topic; absent for the thread's opening. */
  divider_id?: string;
  /** When the topic opened (RFC 3339). */
  started_at: string;
  /** The athlete's first words in the topic; absent while nobody spoke in it. */
  title?: string;
}

interface TopicsResponse {
  topics: ThreadTopic[];
}

export const topicsApi = {
  /**
   * The topics of a thread, newest first: one per divider, plus the
   * thread's opening.
   */
  async getConversationTopics(conversationId: string): Promise<ThreadTopic[]> {
    const response = await axios.get<TopicsResponse>(`/api/chat/conversations/${conversationId}/topics`);
    return response.data.topics;
  },
};
