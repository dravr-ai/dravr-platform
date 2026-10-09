// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Maestro runScript step that asks the control process (frontend/e2e-real/home-sync-control.ts) for an onboarding coach
// ABOUTME: ACTION=coach makes a fresh coach into output.coach; ACTION=state reads what the group step left into output.coachState

// Maestro's script runtime provides these: `http`, `json` and `output`, and
// each env variable of the step (ACTION, EMAIL, STEP) or the run (CONTROL_URL)
// as a global.
/* global ACTION, CONTROL_URL, EMAIL, STEP, http, json, output */

var base = typeof CONTROL_URL === 'undefined' ? 'http://127.0.0.1:8098' : CONTROL_URL;
var response;
if (ACTION === 'coach') {
  response = http.post(base + '/onboarding-coach', { body: '' });
} else if (ACTION === 'state') {
  response = http.get(
    base + '/onboarding-coach/state?email=' + encodeURIComponent(EMAIL) + '&step=' + encodeURIComponent(STEP),
  );
} else {
  throw new Error('coach-control: unknown ACTION ' + ACTION);
}
if (!response.ok) {
  throw new Error('control ' + ACTION + ' answered ' + response.status + ': ' + response.body);
}
if (ACTION === 'coach') {
  output.coach = json(response.body);
} else {
  output.coachState = json(response.body);
}
