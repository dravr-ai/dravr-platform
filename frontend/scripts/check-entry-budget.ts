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
 * 300 KB: the entry was 618 KB when Lighthouse scored mobile performance
 * 63-68 on every page, and 265 KB once the five locale catalogues,
 * chart.js and posthog-js left it (mobile 74-91). The headroom is for
 * features; a library or a catalogue landing in the entry again trips it.
 */
const BUDGET_BYTES = 300 * 1024;

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
