// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Regenerate is offered only on a reply whose turn the server says changed nothing
// ABOUTME: A row that does not say, written before the field existed, offers none

import { describe, it, expect } from 'vitest';
import { canRegenerateReply } from '../src/regenerate';

describe('canRegenerateReply', () => {
  it('offers it when the server says the turn attempted no write', () => {
    expect(canRegenerateReply({ turn_attempted_write: false })).toBe(true);
  });

  it('refuses a turn that was blocked or attempted a write', () => {
    expect(canRegenerateReply({ turn_attempted_write: true })).toBe(false);
  });

  it('refuses a row that does not say', () => {
    expect(canRegenerateReply({ turn_attempted_write: null })).toBe(false);
    expect(canRegenerateReply({})).toBe(false);
  });
});
