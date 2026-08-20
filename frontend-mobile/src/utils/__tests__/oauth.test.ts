// ABOUTME: Pins the OAuth callback URL to the deep-link scheme app.config.js registers.
// ABOUTME: Guards the drift that silently dead-ends every provider connection.

import { getOAuthCallbackUrl } from '../oauth';

// Read the scheme out of the Expo config rather than restating it. A literal
// expectation here would stay green while `scheme` moved underneath it, which
// is exactly the drift that broke provider connections: app.config.js
// registered one scheme and src/utils/oauth.ts asked the server to redirect to
// another. Resolved without the extension so jest's `.js`-stripping
// moduleNameMapper does not rewrite it.
const appConfig = require('../../../app.config') as { scheme?: string };

describe('getOAuthCallbackUrl', () => {
  // The screens that consume this mock it out, so nothing else exercises the
  // real implementation.
  it('builds the callback on the scheme app.config.js registers', () => {
    // Asserted before the comparison so a restructured config (Expo's dynamic
    // function form, say) fails as a missing scheme instead of quietly
    // matching `undefined://oauth-callback` against itself.
    expect(typeof appConfig.scheme).toBe('string');
    expect(getOAuthCallbackUrl()).toBe(`${appConfig.scheme}://oauth-callback`);
  });
});
