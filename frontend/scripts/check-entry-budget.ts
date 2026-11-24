// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Fails the build when the JavaScript a first visit must download before it can paint outgrows its budget
// ABOUTME: Bytes, not a Lighthouse score: the score swings run to run, the gzip size of a given build does not

import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { gzipSync } from 'node:zlib';

/**
 * Gzipped bytes of the entry script plus every chunk index.html preloads.
 *
 * 305 KB: the entry was 618 KB when Lighthouse scored mobile performance
 * 63-68 on every page, and 265 KB once the five locale catalogues,
 * chart.js and posthog-js left it (mobile 74-91). The headroom is for
 * features; a library or a catalogue landing in the entry again trips it.
 * Features had used the first 300 KB by 2026-11-05 (the feedback unread dot,
 * carnet#1392, was the last), and CI builds ~0.2 KB above a local build, so
 * Phil raised it to 305 KB that day; a library or catalogue still costs tens
 * of KB and still trips it. The French catalogue is the default locale and
 * ships in the entry, so every new string grows it: the consent, A2A-form and
 * partial-export copy of 2026-11-06 took the CI build to 305.4 KB, and the
 * budget became 310 KB. The admin's sign-in mode of 2026-11-18 (carnet#1631)
 * added 1.0 KB before first paint: the mode in the session state, the switch
 * in both shells and nine French strings. Its picker loads lazily; the rest
 * runs at first paint, which took the CI build to 310.7 KB, so the budget is
 * 315 KB. The assistant-connection consents of 2026-11-19 (EFVP: the MCP
 * token and API key consents, the coach's AI tools notice, seven French
 * strings through contremaitre 55d6b7ea) took the CI build to 315.4 KB with
 * no code change, so the budget is 320 KB. Main stood at 319.8 KB when the
 * coach's family editor (carnet#1515) landed: its sheet loads lazily on first
 * open, but the plan card's "Build from family" on each day, the draft
 * card's sharper open-draft rule and the API client's catalogue and preview
 * parsers run at first paint, about 1 KB, so the budget is 325 KB. The day
 * editor's sport chips, sets and step targets (carnet#1727) load lazily with
 * the sheet, but their 48 French strings (through contremaitre dcb83215) and
 * the mobility and strength sport labels ship in the entry, which took the CI
 * build to 325.4 KB, so the budget is 330 KB.
 */
const BUDGET_BYTES = 330 * 1024;

const DIST = resolve(import.meta.dirname, '..', 'dist');
const html = readFileSync(join(DIST, 'index.html'), 'utf8');

const entry = [...html.matchAll(/<script[^>]*type="module"[^>]*src="\/([^"]+\.js)"/g)].map((m) => m[1]);
const preloaded = [...html.matchAll(/<link[^>]*rel="modulepreload"[^>]*href="\/([^"]+\.js)"/g)].map((m) => m[1]);

// A build whose shell names no module script is not a small build: the
// pattern above stopped matching, and passing would hide every regression.
if (entry.length === 0) {
  console.error(`check-entry-budget: no module <script src> found in ${join(DIST, 'index.html')}`);
  process.exit(1);
}

let total = 0;
for (const file of [...entry, ...preloaded]) {
  const size = gzipSync(readFileSync(join(DIST, file)), { level: 9 }).length;
  total += size;
  console.log(`${(size / 1024).toFixed(1).padStart(8)} KB  ${file}`);
}
console.log(`${(total / 1024).toFixed(1).padStart(8)} KB  total (budget ${BUDGET_BYTES / 1024} KB, gzip)`);

if (total > BUDGET_BYTES) {
  console.error(
    'check-entry-budget: the entry bundle is over budget. Something now loads before first paint that did ' +
      'not — a static import of a heavy library or of a locale catalogue is the usual cause. Load it where it ' +
      'is used (a lazy route, a dynamic import), or raise BUDGET_BYTES with the reason.',
  );
  process.exit(1);
}
