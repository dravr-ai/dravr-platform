// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the feedback thread parsers — a message with or without the reply it is about, by the athlete or the team
// ABOUTME: Red if a malformed body is read as a thread or a count, which would show the athlete messages they never sent

import { describe, expect, it } from 'vitest';
import {
  isFeedbackImageType,
  parseFeedbackCreated,
  parseFeedbackMessage,
  parseFeedbackThread,
  parseFeedbackUnread,
} from '../src/feedback';

const MESSAGE = {
  id: 'm-1',
  author: 'athlete',
  body: 'The map is slow.',
  created_at: '2026-10-28T15:00:00Z',
  about: null,
  has_image: false,
};
const ABOUT = { conversation_id: 'c-1', message_id: 'r-1' };

describe('parseFeedbackMessage', () => {
  it('reads a message, with or without the reply it is about', () => {
    expect(parseFeedbackMessage(MESSAGE)).toEqual(MESSAGE);
    expect(parseFeedbackMessage({ ...MESSAGE, about: ABOUT })).toEqual({ ...MESSAGE, about: ABOUT });
    const withoutAbout = {
      id: MESSAGE.id,
      author: 'athlete',
      body: MESSAGE.body,
      created_at: MESSAGE.created_at,
      has_image: false,
    };
    expect(parseFeedbackMessage(withoutAbout)).toEqual(MESSAGE);
  });

  it('reads who wrote it, and refuses a message naming neither side', () => {
    expect(parseFeedbackMessage({ ...MESSAGE, author: 'team' })?.author).toBe('team');
    const { author: _author, ...unsigned } = MESSAGE;
    expect(parseFeedbackMessage(unsigned)).toBeNull();
    expect(parseFeedbackMessage({ ...MESSAGE, author: 'coach' })).toBeNull();
  });

  it('reads whether a screenshot came with it, and refuses a message that does not say', () => {
    expect(parseFeedbackMessage({ ...MESSAGE, has_image: true })?.has_image).toBe(true);
    expect(parseFeedbackMessage({ ...MESSAGE, has_image: undefined })).toBeNull();
    expect(parseFeedbackMessage({ ...MESSAGE, has_image: 'yes' })).toBeNull();
  });

  it('rejects a message missing a field or carrying a malformed reply', () => {
    expect(parseFeedbackMessage({ ...MESSAGE, body: 3 })).toBeNull();
    expect(parseFeedbackMessage({ ...MESSAGE, created_at: undefined })).toBeNull();
    expect(parseFeedbackMessage({ ...MESSAGE, about: { conversation_id: 'c-1' } })).toBeNull();
    expect(parseFeedbackMessage({ ...MESSAGE, about: 'c-1' })).toBeNull();
    expect(parseFeedbackMessage(null)).toBeNull();
  });
});

describe('parseFeedbackThread', () => {
  it('reads every message in order', () => {
    const second = { ...MESSAGE, id: 'm-2', about: ABOUT };
    expect(parseFeedbackThread({ messages: [MESSAGE, second] })).toEqual({ messages: [MESSAGE, second] });
    expect(parseFeedbackThread({ messages: [] })).toEqual({ messages: [] });
  });

  it('rejects the whole thread when one message is malformed', () => {
    expect(parseFeedbackThread({ messages: [MESSAGE, { id: 'm-2' }] })).toBeNull();
    expect(parseFeedbackThread({})).toBeNull();
    expect(parseFeedbackThread([MESSAGE])).toBeNull();
  });
});

describe('isFeedbackImageType', () => {
  it('takes the four screenshot types the server reads and nothing else', () => {
    for (const type of ['image/png', 'image/jpeg', 'image/webp', 'image/gif']) {
      expect(isFeedbackImageType(type)).toBe(true);
    }
    for (const type of ['image/svg+xml', 'application/pdf', 'image/heic', '']) {
      expect(isFeedbackImageType(type)).toBe(false);
    }
  });
});

describe('parseFeedbackUnread', () => {
  it('reads a count of waiting replies', () => {
    expect(parseFeedbackUnread({ unread: 0 })).toEqual({ unread: 0 });
    expect(parseFeedbackUnread({ unread: 3 })).toEqual({ unread: 3 });
  });

  it('rejects anything that is not a whole, non-negative count', () => {
    for (const body of [{}, { unread: '2' }, { unread: -1 }, { unread: 1.5 }, null, 2]) {
      expect(parseFeedbackUnread(body)).toBeNull();
    }
  });
});

describe('parseFeedbackCreated', () => {
  it('reads the message a 201 carries', () => {
    expect(parseFeedbackCreated({ message: MESSAGE })).toEqual(MESSAGE);
    expect(parseFeedbackCreated(MESSAGE)).toBeNull();
  });
});
