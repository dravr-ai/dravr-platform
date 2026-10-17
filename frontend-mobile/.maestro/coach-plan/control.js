// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Maestro runScript step that asks the control process (frontend/e2e-real/home-sync-control.ts) for the coach plan-thread state
// ABOUTME: ACTION=thread seeds a fresh coach and athlete into output.plan; ACTION=state reads the athlete's season into output.planState

// Maestro's script runtime provides these: `http`, `json` and `output`, and
// each env variable of the step (ACTION, ATHLETE) or the run (CONTROL_URL) as
// a global.
/* global ACTION, ATHLETE, CONTROL_URL, http, json, output */

var base = typeof CONTROL_URL === 'undefined' ? 'http://127.0.0.1:8098' : CONTROL_URL;
var response;
if (ACTION === 'thread') {
  response = http.post(base + '/coach-plan-thread', { body: '' });
} else if (ACTION === 'state') {
  response = http.get(base + '/coach-plan-thread/state?athlete=' + encodeURIComponent(ATHLETE));
} else {
  throw new Error('coach-plan control: unknown ACTION ' + ACTION);
}
if (!response.ok) {
  throw new Error('control ' + ACTION + ' answered ' + response.status + ': ' + response.body);
}
if (ACTION === 'thread') {
  output.plan = json(response.body);
} else {
  output.planState = json(response.body);
}
