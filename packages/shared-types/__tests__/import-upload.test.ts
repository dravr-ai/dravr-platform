// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The upload target's wire shape: a session with its URL, or through the API, and nothing else reads as either
// ABOUTME: The start's answer carries the import beside its target, so one parser reads both from one body

import { describe, expect, it } from 'vitest';
import { parseImportUploadTarget, parseStartedImportUpload } from '../src/imports';

const IMPORT = {
  id: 'job-1',
  source: 'apple_health_archive',
  status: 'awaiting_link',
  total_rows: null,
  processed_rows: 0,
  imported: 0,
  skipped: 0,
  failed: 0,
  error_code: null,
  earliest_start: null,
  latest_start: null,
  created_at: '2026-11-04T10:00:00Z',
  updated_at: '2026-11-04T10:00:00Z',
  finished_at: null,
  expires_at: '2026-11-10T10:00:00Z',
  history_held: false,
};

describe('parseImportUploadTarget', () => {
  it('reads both targets', () => {
    expect(parseImportUploadTarget({ kind: 'through_api' })).toEqual({ kind: 'through_api' });
    expect(parseImportUploadTarget({ kind: 'session', url: 'https://s/x' })).toEqual({ kind: 'session', url: 'https://s/x' });
  });

  it('refuses a session with no URL, and any other kind', () => {
    for (const body of [{ kind: 'session' }, { kind: 'session', url: '' }, { kind: 'other' }, null, 'session']) {
      expect(parseImportUploadTarget(body)).toBeNull();
    }
  });
});

describe('parseStartedImportUpload', () => {
  it('reads the import beside its target', () => {
    const started = parseStartedImportUpload({ ...IMPORT, upload: { kind: 'through_api' } });
    expect(started?.import.id).toBe('job-1');
    expect(started?.upload).toEqual({ kind: 'through_api' });
  });

  it('refuses a body missing either', () => {
    expect(parseStartedImportUpload(IMPORT)).toBeNull();
    expect(parseStartedImportUpload({ upload: { kind: 'through_api' } })).toBeNull();
  });
});
