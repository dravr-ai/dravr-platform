// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the feedback thread route — `feedback/reply/<conversation>/<message>` round-trips the reply it names
// ABOUTME: Any other shape, or a malformed escape in a hand-typed hash, opens the plain thread

import { describe, it, expect } from 'vitest';
import { FEEDBACK_TAB, feedbackReplyRoute, parseFeedbackAbout } from '../feedbackRoute';

describe('feedback thread route', () => {
  it('round-trips ids that need escaping', () => {
    const route = feedbackReplyRoute('conv 1/2', 'msg?3');
    expect(route).toBe('feedback/reply/conv%201%2F2/msg%3F3');
    expect(parseFeedbackAbout(route.slice(`${FEEDBACK_TAB}/`.length))).toEqual({
      conversation_id: 'conv 1/2',
      message_id: 'msg?3',
    });
  });

  it('names no reply for any other subview', () => {
    expect(parseFeedbackAbout('')).toBeNull();
    expect(parseFeedbackAbout('reply')).toBeNull();
    expect(parseFeedbackAbout('reply/conv-1')).toBeNull();
    expect(parseFeedbackAbout('reply/conv-1/')).toBeNull();
    expect(parseFeedbackAbout('chat/conv-1/msg-1')).toBeNull();
    expect(parseFeedbackAbout('reply/conv-1/msg-1/extra')).toBeNull();
    expect(parseFeedbackAbout('reply/%E0%A4%A/msg-1')).toBeNull();
  });
});
