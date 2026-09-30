// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Maestro runScript step that calls the sciotte double's control API (frontend/e2e-real/home-sync-control.ts)
// ABOUTME: ACTION names the call; its JSON answer is merged into output.homeSync, so setup's names outlive later calls

// Maestro's script runtime provides these: `http`, `json` and `output`, and
// each env variable of the step (ACTION) or the run (CONTROL_URL) as a global.
/* global ACTION, CONTROL_URL, http, json, output */

var base = typeof CONTROL_URL === 'undefined' ? 'http://127.0.0.1:8098' : CONTROL_URL;
var response =
  ACTION === 'reads' ? http.get(base + '/reads') : http.post(base + '/' + ACTION, { body: '' });
if (!response.ok) {
  throw new Error('control ' + ACTION + ' answered ' + response.status + ': ' + response.body);
}
var answer = json(response.body);
var merged = output.homeSync || {};
for (var key in answer) {
  merged[key] = answer[key];
}
output.homeSync = merged;
