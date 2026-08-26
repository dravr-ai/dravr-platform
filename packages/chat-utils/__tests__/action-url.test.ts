// ABOUTME: Unit tests for the trusted-domain gate every `url` reply action passes through
// ABOUTME: Red when the allowlist is widened, dropped, or fooled by a lookalike hostname

import { describe, it, expect } from 'vitest';
import { trustedActionUrl } from '../src/action-url';

const APP_ORIGIN = 'https://app.example.test';

describe('trustedActionUrl', () => {
  it('vouches for a provider OAuth host and returns the URL to open', () => {
    expect(
      trustedActionUrl('https://www.strava.com/oauth/authorize?client_id=1&scope=read'),
    ).toBe('https://www.strava.com/oauth/authorize?client_id=1&scope=read');
  });

  it('vouches for the running app\'s own origin, which mints reconnect links', () => {
    expect(
      trustedActionUrl(`${APP_ORIGIN}/providers/sciotte/login?token=abc`, [APP_ORIGIN]),
    ).toBe(`${APP_ORIGIN}/providers/sciotte/login?token=abc`);
  });

  it('refuses a foreign host outright', () => {
    expect(trustedActionUrl('https://attacker.example/steal')).toBeNull();
  });

  it('refuses a lookalike that merely ends with a trusted name', () => {
    // The open-redirect classic: a substring test would accept this.
    expect(trustedActionUrl('https://strava.com.attacker.example/steal')).toBeNull();
    expect(trustedActionUrl('https://notstrava.com/oauth/authorize')).toBeNull();
  });

  it('refuses a non-HTTP scheme', () => {
    expect(trustedActionUrl('javascript:alert(1)')).toBeNull();
    expect(trustedActionUrl('data:text/html,<script>alert(1)</script>')).toBeNull();
  });

  it('refuses a relative path and an unparseable value', () => {
    expect(trustedActionUrl('/settings/connections')).toBeNull();
    expect(trustedActionUrl('not a url at all')).toBeNull();
  });

  it('does not treat a malformed app origin as a wildcard', () => {
    expect(trustedActionUrl('https://attacker.example/steal', ['', 'nonsense'])).toBeNull();
  });
});
