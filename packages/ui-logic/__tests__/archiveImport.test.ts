// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves both clients say the same thing about an import: its state line, its screen's phase, each code's copy
// ABOUTME: Every key the copy names exists in every shipped locale, so no state renders as a raw key

import { describe, it, expect } from 'vitest';
import de from '../../i18n/src/locales/de/translation.json';
import en from '../../i18n/src/locales/en/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import type { ArchiveImport, ImportStatus } from '@pierre/shared-types';
import {
  AWAITING_LINK_LATE_MS,
  importDeleteCopy,
  importFailureKey,
  importHistoryCopy,
  importOutcomeKey,
  importPhase,
  importSize,
  importStateCopy,
  linkRefusalKey,
  startErrorKey,
} from '../src/archiveImport';

const LOCALES: Record<string, unknown> = { de, en, es, fr, pt };

function lookup(catalogue: unknown, key: string): unknown {
  return key.split('.').reduce<unknown>(
    (node, part) => (typeof node === 'object' && node !== null ? (node as Record<string, unknown>)[part] : undefined),
    catalogue,
  );
}

function entry(status: ImportStatus, overrides: Partial<ArchiveImport> = {}): ArchiveImport {
  return {
    id: 'job-1',
    source: 'strava_archive',
    status,
    total_rows: 3105,
    processed_rows: 1240,
    imported: 1240,
    skipped: 0,
    failed: 0,
    error_code: null,
    earliest_start: '2009-03-01T08:00:00Z',
    latest_start: '2026-03-01T08:00:00Z',
    created_at: '2026-10-27T08:00:00Z',
    updated_at: '2026-10-27T08:00:00Z',
    finished_at: '2026-10-27T09:00:00Z',
    expires_at: null,
    history_held: true,
    ...overrides,
  };
}

describe('an import, as both clients say it', () => {
  it('gives each state its line, and a late link its own', () => {
    const created = Date.parse('2026-10-27T08:00:00Z');
    expect(importStateCopy(entry('awaiting_link'), 'en', created + 1000).key).toBe('providers.import.state.awaitingLink');
    expect(importStateCopy(entry('awaiting_link'), 'en', created + AWAITING_LINK_LATE_MS + 1).key).toBe(
      'providers.import.state.awaitingLinkLate',
    );
    expect(importStateCopy(entry('importing'), 'en')).toEqual({
      key: 'providers.import.state.importing',
      values: { processed: 1240, total: 3105 },
    });
    expect(importStateCopy(entry('completed'), 'en')).toMatchObject({
      key: 'providers.import.state.completedN',
      values: { count: 1240 },
    });
    expect(importStateCopy(entry('failed'), 'en').key).toBe('providers.import.state.stoppedN');
    expect(importStateCopy(entry('cancelled'), 'en').key).toBe('providers.import.state.stoppedN');
  });

  // carnet#1212: the count read one plural form, so « 1 activities imported »
  // and a French « Import arrêté · 0 importées »; countKey now picks the form.
  it('picks the imported count\'s form in the reader\'s language', () => {
    expect(importStateCopy(entry('completed', { imported: 1 }), 'en').key).toBe('providers.import.state.completedOne');
    expect(importStateCopy(entry('completed', { imported: 0 }), 'en').key).toBe('providers.import.state.completedN');
    expect(importStateCopy(entry('completed', { imported: 0 }), 'fr').key).toBe('providers.import.state.completedOne');
    expect(importStateCopy(entry('failed', { imported: 0 }), 'pt-BR')).toEqual({
      key: 'providers.import.state.stoppedOne',
      values: { count: 0 },
    });
    expect(importStateCopy(entry('cancelled', { imported: 0 }), 'de').key).toBe('providers.import.state.stoppedN');
  });

  // carnet#1212: the history line and the delete confirmation read one plural
  // form, so « Delete 1 activities »; both clients now read these helpers.
  it('counts the held history and the deletion in the reader\'s language', () => {
    expect(importHistoryCopy(entry('completed', { imported: 1 }), 'en')).toEqual({
      key: 'providers.import.strava.historyLineOne',
      values: { count: 1, date: expect.any(String) },
    });
    expect(importHistoryCopy(entry('completed'), 'fr').key).toBe('providers.import.strava.historyLineN');
    expect(importDeleteCopy(1, 'en')).toEqual({
      body: { key: 'providers.import.strava.deleteConfirmBodyOne', values: { count: 1 } },
      action: { key: 'providers.import.strava.deleteConfirmActionOne', values: { count: 1 } },
    });
    expect(importDeleteCopy(0, 'fr').action.key).toBe('providers.import.strava.deleteConfirmActionOne');
    expect(importDeleteCopy(0, 'es').action.key).toBe('providers.import.strava.deleteConfirmActionN');
    expect(importDeleteCopy(1240, 'de').body.key).toBe('providers.import.strava.deleteConfirmBodyN');
  });

  it('puts the screen in the phase the latest import calls for', () => {
    expect(importPhase(null)).toBe('request');
    expect(importPhase(entry('awaiting_link'))).toBe('awaiting_link');
    expect(importPhase(entry('queued'))).toBe('running');
    expect(importPhase(entry('downloading'))).toBe('running');
    expect(importPhase(entry('importing'))).toBe('running');
    expect(importPhase(entry('completed'))).toBe('completed');
    expect(importPhase(entry('failed')), 'a failed import asks for the link again').toBe('request');
    expect(
      importPhase(entry('completed', { history_held: false })),
      'a completed import whose history was deleted asks for an export again',
    ).toBe('request');
    expect(importStateCopy(entry('completed', { history_held: false }), 'en').key).toBe(
      'providers.import.strava.deleted',
    );
  });

  it('reads a refused start by the code the server names, and a second import as in progress', () => {
    expect(startErrorKey(400, 'link_host_not_allowed: the export link was refused')).toBe(
      'providers.import.strava.refusal.link_host_not_allowed',
    );
    expect(startErrorKey(409, 'an import of strava_archive in progress')).toBe('providers.import.strava.inProgress');
    expect(startErrorKey(500, 'boom')).toBe('providers.import.strava.genericError');
  });

  it('reads an unknown failure or outcome as the generic copy rather than a raw key', () => {
    expect(importFailureKey('spool_missing')).toBe('providers.import.strava.failure.spool_missing');
    expect(importFailureKey('something_new')).toBe('providers.import.strava.genericError');
    expect(importOutcomeKey('something_new')).toBe('providers.import.strava.outcome.unreadable_file');
  });

  it('states a size the way an athlete reads one', () => {
    expect(importSize(512)).toBe('512 B');
    expect(importSize(1_200_000_000)).toBe('1.2 GB');
  });

  it('names only keys every shipped locale carries', () => {
    const keys = [
      ...(['awaiting_link', 'queued', 'downloading', 'importing', 'completed', 'failed'] as ImportStatus[]).map(
        (status) => importStateCopy(entry(status), 'en').key,
      ),
      ...(['completed', 'failed'] as ImportStatus[]).map(
        (status) => importStateCopy(entry(status, { imported: 1 }), 'en').key,
      ),
      ...[1, 2].flatMap((count) => {
        const { body, action } = importDeleteCopy(count, 'en');
        return [body.key, action.key, importHistoryCopy(entry('completed', { imported: count }), 'en').key];
      }),
      'providers.import.state.awaitingLinkLate',
      ...(['link_invalid', 'link_host_not_allowed', 'link_expired', 'link_unreachable'] as const).map(linkRefusalKey),
      ...[
        'download_failed',
        'link_expired',
        'archive_too_large',
        'archive_unreadable',
        'archive_limit',
        'missing_activities_csv',
        'unknown_locale',
        'spool_missing',
        'not_an_export',
        null,
      ].map(importFailureKey),
      ...['strava_export_error', 'missing_file', 'unreadable_file', 'file_too_large', 'invalid_row', 'zero_elapsed'].map(
        importOutcomeKey,
      ),
      'providers.import.strava.inProgress',
    ];
    for (const [locale, catalogue] of Object.entries(LOCALES)) {
      for (const key of keys) {
        expect(typeof lookup(catalogue, key), `${locale}: ${key}`).toBe('string');
      }
    }
  });
});
