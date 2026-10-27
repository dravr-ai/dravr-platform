// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Maestro runScript step that asks the control process (frontend/e2e-real/home-sync-control.ts) for a session minted for EMAIL
// ABOUTME: Puts the seeded-session link's query (a single-use code, never a token) into output.seededSession.query; the caller opens it

// Maestro's script runtime provides these: `http`, `json` and `output`, and
// each env variable of the step (EMAIL) or the run (CONTROL_URL) as a global.
/* global CONTROL_URL, EMAIL, http, json, output */

var base = typeof CONTROL_URL === 'undefined' ? 'http://127.0.0.1:8098' : CONTROL_URL;
if (typeof EMAIL === 'undefined' || EMAIL === '') {
  throw new Error('sign-in-seeded: EMAIL names the account to sign in');
}
var response = http.post(base + '/seeded-session?email=' + encodeURIComponent(EMAIL), { body: '' });
if (!response.ok) {
  // The body names what the CLI refused; it never carries a token.
  throw new Error('control seeded-session answered ' + response.status + ': ' + response.body);
}
// The control process holds the minted pair and answers with a code the app
// redeems once, within a minute: Maestro writes every link it opens to its
// log, which the lanes upload, so the link must not carry the tokens.
var code = json(response.body).code;
if (typeof code !== 'string' || code === '') {
  throw new Error('control seeded-session answered no code');
}
output.seededSession = { query: 'code=' + encodeURIComponent(code) };
