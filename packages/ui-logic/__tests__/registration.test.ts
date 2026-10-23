// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the sentence each registration outcome greets a new account with
// ABOUTME: One translation key per outcome, so web and mobile word it the same in every language

import { describe, expect, it } from 'vitest';
import { describeRegistration } from '../src/registration';

const t = (key: string) => key;

describe('describeRegistration', () => {
  it('names what the account waits on, one key per outcome', () => {
    expect(describeRegistration({ activation: 'active' }, t)).toBe('auth.registeredReady');
    expect(describeRegistration({ activation: 'confirm_email' }, t)).toBe('auth.registeredConfirmEmail');
    expect(describeRegistration({ activation: 'review' }, t)).toBe('auth.registeredReview');
  });
});
