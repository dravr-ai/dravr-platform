// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the chat-app offer outside the wizard — the record its answer lives on, and the app it names
// ABOUTME: Pins that « Pas maintenant » and a landed link both answer it, and that the recommended app is the one named

import { describe, expect, it } from 'vitest';
import { CHAT_APP_OFFER_STEP_ID, ONBOARDING_STEPS, chatAppOfferAnswered, offeredChatApp } from '../src';

describe('chatAppOfferAnswered', () => {
  it('reads « Pas maintenant » and a landed link as answered, on the record the wizard wrote', () => {
    // A persisted value: accounts that answered the wizard's chat-app screen
    // already hold `messaging_configure` rows, and the offer must read them.
    expect(CHAT_APP_OFFER_STEP_ID).toBe('messaging_configure');
    expect(chatAppOfferAnswered([{ step_id: 'messaging_configure', status: 'skipped' }])).toBe(true);
    expect(chatAppOfferAnswered([{ step_id: 'messaging_configure', status: 'complete' }])).toBe(true);
  });

  it('leaves the offer open with no record, another step’s record, or one never answered', () => {
    expect(chatAppOfferAnswered(undefined)).toBe(false);
    expect(chatAppOfferAnswered([])).toBe(false);
    expect(chatAppOfferAnswered([{ step_id: 'messaging_channel', status: 'skipped' }])).toBe(false);
    expect(chatAppOfferAnswered([{ step_id: 'messaging_configure', status: 'not_applicable' }])).toBe(false);
  });

  it('is no wizard step any more (carnet#1525)', () => {
    expect(ONBOARDING_STEPS.map((s) => s.id)).not.toContain(CHAT_APP_OFFER_STEP_ID);
  });
});

describe('offeredChatApp', () => {
  const telegram = { channel: 'telegram', recommended: true };
  const whatsapp = { channel: 'whatsapp', recommended: false };

  it('names the recommended app, else the first offered, else none', () => {
    expect(offeredChatApp([whatsapp, telegram])).toBe(telegram);
    expect(offeredChatApp([whatsapp])).toBe(whatsapp);
    expect(offeredChatApp([])).toBeUndefined();
  });
});
