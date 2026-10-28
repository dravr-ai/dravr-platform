// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the feedback thread parsers — a message with or without the reply it is about reads through
// ABOUTME: Red if a malformed body is read as a thread, which would show the athlete messages they never sent

import { describe, expect, it } from 'vitest';
import { parseFeedbackCreated, parseFeedbackMessage, parseFeedbackThread } from '../src/feedback';

const MESSAGE = { id: 'm-1', body: 'The map is slow.', created_at: '2026-10-28T15:00:00Z', about: null };
const ABOUT = { conversation_id: 'c-1', message_id: 'r-1' };

describe('parseFeedbackMessage', () => {
  it('reads a message, with or without the reply it is about', () => {
    expect(parseFeedbackMessage(MESSAGE)).toEqual(MESSAGE);
    expect(parseFeedbackMessage({ ...MESSAGE, about: ABOUT })).toEqual({ ...MESSAGE, about: ABOUT });
    const withoutAbout = { id: MESSAGE.id, body: MESSAGE.body, created_at: MESSAGE.created_at };
    expect(parseFeedbackMessage(withoutAbout)).toEqual(MESSAGE);
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

describe('parseFeedbackCreated', () => {
  it('reads the message a 201 carries', () => {
    expect(parseFeedbackCreated({ message: MESSAGE })).toEqual(MESSAGE);
    expect(parseFeedbackCreated(MESSAGE)).toBeNull();
  });
});
