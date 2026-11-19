// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves the end of a data export is read as the server writes it: whole or cut, complete or partial, and why
// ABOUTME: The tails are the server's own closing fields (crates/pierre-routes-auth/src/data_export.rs)

import { describe, it, expect } from 'vitest';
import {
  EXPORT_PARTIAL_KEYS,
  ORIGINALS_KEYS,
  SEAL_MAX_CHARS,
  SEAL_MIN_CHARS,
  exportPartialKey,
  exportRefusalCode,
  originalsIncomplete,
  originalsRefusalKey,
  originalsStateKey,
  originalsStillBuilding,
  readExportTail,
  sealPassphraseAccepted,
} from '../src/dataExport';

const ROWS = '{"format":"dravr-user-export","tables":{"users":[{"id":"a","note":"complete"}]';

/** The closing fields as the server writes them; `limit` undefined is a server that predates `limit_reached`. */
function end(limit: string | undefined, failed: string[], skipped: string[]): string {
  const limitField = limit === undefined ? '' : `"limit_reached":${limit},`;
  return `${ROWS}]},"complete":${failed.length + skipped.length === 0},${limitField}"failed_tables":${JSON.stringify(
    failed,
  )},"skipped_tables":${JSON.stringify(skipped)}}`;
}

describe('readExportTail', () => {
  it('reads a complete document', () => {
    const tail = readExportTail(
      `${ROWS}},"complete":true,"limit_reached":null,"failed_tables":[],"skipped_tables":[]}`,
    );
    expect(tail).toEqual({
      complete: true,
      limitReached: null,
      hasFailedTables: false,
      hasSkippedTables: false,
    });
    expect(exportPartialKey(tail!)).toBeNull();
  });

  it('reads the limit and both table lists of a partial document', () => {
    const tail = readExportTail(
      `${ROWS}]},"complete":false,"limit_reached":"time","failed_tables":["recipes"],"skipped_tables":["chat_messages","activity_streams"]}\n`,
    );
    expect(tail).toEqual({
      complete: false,
      limitReached: 'time',
      hasFailedTables: true,
      hasSkippedTables: true,
    });
  });

  it('reads a document from a server that does not name the limit', () => {
    const tail = readExportTail(
      `${ROWS}]},"complete":false,"failed_tables":[],"skipped_tables":["recipes","goals"]}`,
    );
    expect(tail).toEqual({
      complete: false,
      limitReached: null,
      hasFailedTables: false,
      hasSkippedTables: true,
    });
  });

  it('takes a file that stops inside its rows for one cut short', () => {
    expect(readExportTail(ROWS)).toBeNull();
    expect(readExportTail(`${ROWS},"complete":false`)).toBeNull();
  });
});

describe('exportPartialKey', () => {
  it.each([
    ['the size limit', '"size"', [], ['activity_streams'], 'dataExport.partialSize'],
    ['the time limit', '"time"', [], ['chat_messages'], 'dataExport.partialTime'],
    [
      'the size limit and an unread table',
      '"size"',
      ['recipes'],
      ['activity_streams'],
      'dataExport.partialSizeAndFailed',
    ],
    [
      'the time limit and an unread table',
      '"time"',
      ['recipes'],
      ['activity_streams'],
      'dataExport.partialTimeAndFailed',
    ],
    ['an unread table and no limit', 'null', ['recipes'], [], 'dataExport.partial'],
    [
      'an unread table from a server that names no limit',
      undefined,
      ['recipes'],
      [],
      'dataExport.partial',
    ],
    ['an unnamed limit', undefined, [], ['recipes', 'goals'], 'dataExport.partialLimit'],
    [
      'an unnamed limit and an unread table',
      undefined,
      ['recipes'],
      ['goals'],
      'dataExport.partialLimitAndFailed',
    ],
  ] as const)('names %s', (_case, limit, failed, skipped, key) => {
    const tail = readExportTail(end(limit, [...failed], [...skipped]));
    expect(tail).not.toBeNull();
    expect(exportPartialKey(tail!)).toBe(key);
    expect(EXPORT_PARTIAL_KEYS).toContain(key);
  });
});

describe('the originals bundle', () => {
  const ready = { status: 'ready' as const, missing: 0, damaged: 0 };

  it('accepts no passphrase, or one of 12 to 1024 characters counted as the server counts them', () => {
    expect(sealPassphraseAccepted('')).toBe(true);
    expect(sealPassphraseAccepted('x'.repeat(SEAL_MIN_CHARS - 1))).toBe(false);
    expect(sealPassphraseAccepted('x'.repeat(SEAL_MIN_CHARS))).toBe(true);
    expect(sealPassphraseAccepted('x'.repeat(SEAL_MAX_CHARS + 1))).toBe(false);
    // Eleven emoji are eleven characters, though many more UTF-16 units.
    expect(sealPassphraseAccepted('🏃'.repeat(11))).toBe(false);
    expect(sealPassphraseAccepted('🏃'.repeat(12))).toBe(true);
  });

  it('says what each state is, and polls only while it builds', () => {
    expect(originalsStateKey({ ...ready, status: 'building' })).toBe('dataExport.originalsBuilding');
    expect(originalsStateKey({ ...ready, status: 'failed' })).toBe('dataExport.originalsFailed');
    expect(originalsStateKey(ready)).toBe('dataExport.originalsReady');
    expect(originalsStillBuilding({ ...ready, status: 'building' })).toBe(true);
    expect(originalsStillBuilding(ready)).toBe(false);
    expect(originalsStillBuilding(null)).toBe(false);
  });

  it('says a ready bundle lacks files only when its manifest names some', () => {
    expect(originalsIncomplete(ready)).toBe(false);
    expect(originalsIncomplete({ ...ready, missing: 2 })).toBe(true);
    expect(originalsIncomplete({ ...ready, damaged: 1 })).toBe(true);
    expect(originalsIncomplete({ status: 'building', missing: null, damaged: null })).toBe(false);
  });

  it("reads a refusal's `error` code from its body, and nothing else", () => {
    expect(exportRefusalCode({ response: { status: 409, data: { error: 'bundle_building', message: 'prose' } } })).toBe(
      'bundle_building',
    );
    expect(exportRefusalCode({ response: { data: { code: 'QuotaExceeded' } } })).toBeUndefined();
    expect(exportRefusalCode({ response: { data: { error: 42 } } })).toBeUndefined();
    expect(exportRefusalCode(new Error('network'))).toBeUndefined();
    expect(exportRefusalCode(null)).toBeUndefined();
  });

  it('reads a refusal by its status and code, never its prose', () => {
    expect(originalsRefusalKey(429, 'too_many_bundles')).toBe('dataExport.originalsRateLimited');
    expect(originalsRefusalKey(503, 'export_busy')).toBe('dataExport.originalsBusy');
    expect(originalsRefusalKey(409, 'bundle_building')).toBe('dataExport.originalsOtherBuilding');
    expect(originalsRefusalKey(400, undefined)).toBe('dataExport.originalsSealTooShort');
    expect(originalsRefusalKey(503, undefined)).toBe('dataExport.originalsFailed');
    expect(originalsRefusalKey(undefined, undefined)).toBe('dataExport.originalsFailed');
  });

  it('names every sentence it can answer with in the one list a catalogue check reads', () => {
    const answers = [
      originalsStateKey({ ...ready, status: 'building' }),
      originalsStateKey({ ...ready, status: 'failed' }),
      originalsStateKey(ready),
      originalsRefusalKey(429, undefined),
      originalsRefusalKey(503, 'export_busy'),
      originalsRefusalKey(409, 'bundle_building'),
      originalsRefusalKey(400, undefined),
    ];
    for (const key of answers) expect(ORIGINALS_KEYS).toContain(key);
  });
});
