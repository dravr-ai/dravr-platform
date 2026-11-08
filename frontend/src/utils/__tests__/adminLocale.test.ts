// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A failed call on an admin-only screen reads one English line in any session
// ABOUTME: Transport failures are named, the server's detail shows only where it is safe, the rest is the fallback

import { describe, it, expect } from 'vitest';
import { describeAdminError } from '../adminLocale';

function refusal(status: number, message?: string) {
  return { response: { status, data: { message } } };
}

describe('describeAdminError', () => {
  it('names a request that never reached a server', () => {
    expect(describeAdminError({ code: 'ERR_NETWORK' })).toBe('Network error. Check your connection.');
  });

  it('shows the detail of a refusal', () => {
    expect(describeAdminError(refusal(403, 'Permission required: manage_configuration'))).toBe(
      'Permission required: manage_configuration',
    );
  });

  it('never shows the detail of a server failure', () => {
    expect(describeAdminError(refusal(500, 'panic at the disco'))).toBe('Server error. Try again a bit later.');
  });

  it('says the fallback when the refusal carries no detail', () => {
    expect(describeAdminError(refusal(403), 'Export failed')).toBe('Export failed');
    expect(describeAdminError(refusal(403))).toBe('Something went wrong');
  });
});
