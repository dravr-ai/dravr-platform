// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves the end of a data export is read as the server writes it: whole or cut, complete or partial, and why
// ABOUTME: The tails are the server's own closing fields (crates/pierre-routes-auth/src/data_export.rs)

import { describe, it, expect } from 'vitest';
import { EXPORT_PARTIAL_KEYS, exportPartialKey, readExportTail } from '../src/dataExport';

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
