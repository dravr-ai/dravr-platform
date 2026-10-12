// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves how an Intervals.icu card connects: the authorization only while the server offers it
// ABOUTME: And that a card linked by API key (a coach's) always reconnects through the key form

import { describe, it, expect } from 'vitest';
import { intervalsConnectPath, type IntervalsConnectCard } from '../src/providers';

const card = (overrides: Partial<IntervalsConnectCard>): IntervalsConnectCard => ({
  requires_oauth: true,
  own_app_required: false,
  connected: false,
  ...overrides,
});

describe('intervalsConnectPath', () => {
  it('offers the choice for a new link while a client resolves', () => {
    expect(intervalsConnectPath(card({}))).toBe('choose');
  });

  it('keeps the key form alone when no client resolves', () => {
    expect(intervalsConnectPath(card({ own_app_required: true }))).toBe('key');
    expect(intervalsConnectPath(card({ requires_oauth: false, own_app_required: false }))).toBe('key');
  });

  it('reconnects a key-linked card through the key form, even with the authorization offered', () => {
    expect(intervalsConnectPath(card({ connected: true, connection_type: 'manual' }))).toBe('key');
  });

  it('reconnects an authorized card straight through the authorization', () => {
    expect(intervalsConnectPath(card({ connected: true, connection_type: 'oauth' }))).toBe('authorize');
  });

  it('sends an authorized card to the key form once no client resolves', () => {
    expect(
      intervalsConnectPath(card({ connected: true, connection_type: 'oauth', own_app_required: true })),
    ).toBe('key');
  });
});
