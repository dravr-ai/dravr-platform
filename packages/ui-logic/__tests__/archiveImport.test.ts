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
  APPLE_HEALTH_COPY,
  IMPORT_SOURCE_COPY,
  uploadErrorKey,
  uploadPercent,
  importDeleteCopy,
  importFailureKey,
  importHistoryCopy,
  importOutcomeKey,
  importPhase,
  importSize,
  importSourceCopy,
  importStateCopy,
  importedHistoryCount,
  linkRefusalKey,
  offeredImportSources,
  startErrorKey,
  type ImportSourceCopy,
} from '../src/archiveImport';

function copyOf(source: string): ImportSourceCopy {
  const copy = importSourceCopy(source);
  if (!copy) throw new Error(`no copy for ${source}`);
  return copy;
}
const STRAVA = copyOf('strava_archive');
const GARMIN = copyOf('garmin_archive');

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
    expect(importStateCopy(entry('awaiting_link'), STRAVA, 'en', created + 1000).key).toBe('providers.import.state.awaitingLink');
    expect(importStateCopy(entry('awaiting_link'), STRAVA, 'en', created + AWAITING_LINK_LATE_MS + 1).key).toBe(
      'providers.import.state.awaitingLinkLate',
    );
    expect(importStateCopy(entry('importing'), STRAVA, 'en')).toEqual({
      key: 'providers.import.state.importing',
      values: { processed: 1240, total: 3105 },
    });
    expect(importStateCopy(entry('completed'), STRAVA, 'en')).toMatchObject({
      key: 'providers.import.state.completedN',
      values: { count: 1240 },
    });
    expect(importStateCopy(entry('failed'), STRAVA, 'en').key).toBe('providers.import.state.stoppedN');
    expect(importStateCopy(entry('cancelled'), STRAVA, 'en').key).toBe('providers.import.state.stoppedN');
  });

  // carnet#1212: the count read one plural form, so « 1 activities imported »
  // and a French « Import arrêté · 0 importées »; countKey now picks the form.
  it('picks the imported count\'s form in the reader\'s language', () => {
    expect(importStateCopy(entry('completed', { imported: 1 }), STRAVA, 'en').key).toBe('providers.import.state.completedOne');
    expect(importStateCopy(entry('completed', { imported: 0 }), STRAVA, 'en').key).toBe('providers.import.state.completedN');
    expect(importStateCopy(entry('completed', { imported: 0 }), STRAVA, 'fr').key).toBe('providers.import.state.completedOne');
    expect(importStateCopy(entry('failed', { imported: 0 }), STRAVA, 'pt-BR')).toEqual({
      key: 'providers.import.state.stoppedOne',
      values: { count: 0 },
    });
    expect(importStateCopy(entry('cancelled', { imported: 0 }), STRAVA, 'de').key).toBe('providers.import.state.stoppedN');
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
    expect(importStateCopy(entry('completed', { history_held: false }), STRAVA, 'en').key).toBe(
      'providers.import.strava.deleted',
    );
  });

  it('reads a refused start by the code the server names, and a second import as in progress', () => {
    expect(startErrorKey(400, 'link_host_not_allowed: the export link was refused', STRAVA)).toBe(
      'providers.import.strava.refusal.link_host_not_allowed',
    );
    expect(startErrorKey(409, 'an import of strava_archive in progress', STRAVA)).toBe(
      'providers.import.strava.inProgress',
    );
    expect(startErrorKey(500, 'boom', STRAVA)).toBe('providers.import.strava.genericError');
  });

  // carnet#1337: the Garmin export is offered beside Strava's; every line
  // that names the service names Garmin, and the rest is shared.
  it('names Garmin wherever a Garmin import\'s copy names the service', () => {
    const created = Date.parse('2026-10-27T08:00:00Z');
    const garmin = entry('awaiting_link', { source: 'garmin_archive' });
    expect(importStateCopy(garmin, GARMIN, 'en', created + 1000).key).toBe('providers.import.garmin.awaitingLink');
    expect(importStateCopy(garmin, GARMIN, 'en', created + AWAITING_LINK_LATE_MS + 1).key).toBe(
      'providers.import.garmin.awaitingLinkLate',
    );
    expect(startErrorKey(400, 'link_expired: the export link was refused', GARMIN)).toBe(
      'providers.import.garmin.refusal.link_expired',
    );
    expect(startErrorKey(409, 'an import of garmin_archive in progress', GARMIN)).toBe(
      'providers.import.garmin.inProgress',
    );
    expect(linkRefusalKey('link_unreachable', GARMIN), 'an unreachable link names no service').toBe(
      'providers.import.strava.refusal.link_unreachable',
    );
    expect(importFailureKey('not_an_export', GARMIN)).toBe('providers.import.garmin.failure.not_an_export');
    expect(importFailureKey('archive_limit', GARMIN)).toBe('providers.import.strava.failure.archive_limit');
    expect(GARMIN.exportPage).toMatch(/^https:\/\/www\.garmin\.com\//);
  });

  it('lists the offered sources it can show, in the server\'s order', () => {
    expect(offeredImportSources(['strava_archive', 'garmin_archive']).map((copy) => copy.source)).toEqual([
      'strava_archive',
      'garmin_archive',
    ]);
    expect(offeredImportSources(['garmin_archive']).map((copy) => copy.name)).toEqual([
      'providers.import.sources.garmin',
    ]);
    expect(offeredImportSources(['fitbit_archive']), 'a source with no copy is not shown').toEqual([]);
  });

  it('reads an unknown failure or outcome as the generic copy rather than a raw key', () => {
    expect(importFailureKey('spool_missing', STRAVA)).toBe('providers.import.strava.failure.spool_missing');
    expect(importFailureKey('something_new', STRAVA)).toBe('providers.import.strava.genericError');
    expect(importOutcomeKey('something_new')).toBe('providers.import.strava.outcome.unreadable_file');
  });

  it('states a size the way an athlete reads one', () => {
    expect(importSize(512)).toBe('512 B');
    expect(importSize(1_200_000_000)).toBe('1.2 GB');
  });

  it('names a deleted activity\'s row rather than calling it unreadable', () => {

    expect(importOutcomeKey('deleted_by_athlete')).toBe('providers.import.strava.outcome.deleted_by_athlete');

  });


  it('blames no link for an upload that stopped while it was read', () => {
    expect(importFailureKey('link_expired', APPLE_HEALTH_COPY)).toBe('providers.import.strava.genericError');
    expect(importFailureKey('link_expired', GARMIN)).toBe('providers.import.strava.failure.link_expired');
    expect(importFailureKey('archive_too_large', APPLE_HEALTH_COPY)).toBe('providers.import.strava.failure.archive_too_large');
  });

  it('lists the sources an athlete uploads beside those they link', () => {
    const offered = ['strava_archive', 'apple_health_archive', 'coros_archive', 'whoop_archive'];
    expect(offeredImportSources(offered).map((copy) => [copy.source, copy.intake])).toEqual([
      ['strava_archive', 'link'],
      ['apple_health_archive', 'upload'],
      ['coros_archive', 'upload'],
      ['whoop_archive', 'upload'],
    ]);
    expect(importSourceCopy('apple_health_archive')).toBe(APPLE_HEALTH_COPY);
  });

  it('names each upload source in its own strings', () => {
    for (const service of ['coros', 'whoop']) {
      const copy = importSourceCopy(`${service}_archive`);
      expect(copy?.name, service).toBe(`providers.import.sources.${service}`);
      expect(copy?.notAnExport, service).toBe(`providers.import.${service}.failure.not_an_export`);
    }
  });

  it('tells a WHOOP athlete why its columns were refused, and no other source', () => {
    const whoop = importSourceCopy('whoop_archive');
    const coros = importSourceCopy('coros_archive');
    if (!whoop || !coros) throw new Error('both sources have copy');
    expect(importFailureKey('unknown_columns', whoop)).toBe('providers.import.whoop.failure.unknown_columns');
    expect(importFailureKey('unknown_columns', coros)).toBe('providers.import.strava.genericError');
  });

  it('says an upload waits for its file, not for an email', () => {
    const line = importStateCopy(entry('awaiting_link', { source: 'apple_health_archive' }), APPLE_HEALTH_COPY, 'en');
    expect(line.key).toBe('providers.import.apple.awaitingUpload');
  });

  it('never names a link refusal of a source that has no link', () => {
    expect(linkRefusalKey('link_expired', APPLE_HEALTH_COPY)).toBe('providers.import.strava.refusal.link_unreachable');
  });

  it('names why an upload stopped, by the failure it carries or the status it answered', () => {
    expect(uploadErrorKey({ failure: 'stalled' }, APPLE_HEALTH_COPY)).toBe(APPLE_HEALTH_COPY.stalled);
    expect(uploadErrorKey({ failure: 'gone' }, APPLE_HEALTH_COPY)).toBe(APPLE_HEALTH_COPY.gone);
    expect(uploadErrorKey({ response: { status: 409, data: {} } }, APPLE_HEALTH_COPY)).toBe(APPLE_HEALTH_COPY.inProgress);
    expect(uploadErrorKey({ response: { status: 413, data: {} } }, APPLE_HEALTH_COPY)).toBe(
      'providers.import.strava.failure.archive_too_large',
    );
    expect(uploadErrorKey({ failure: 'refused' }, APPLE_HEALTH_COPY)).toBe('providers.import.strava.genericError');
    expect(uploadErrorKey(new Error('network'), APPLE_HEALTH_COPY)).toBe('providers.import.strava.genericError');
  });

  it('reads 100% only once the whole file has landed', () => {
    expect(uploadPercent(0, 1000)).toBe(0);
    expect(uploadPercent(999, 1000)).toBe(99);
    expect(uploadPercent(1000, 1000)).toBe(100);
    expect(uploadPercent(0, 0)).toBe(0);
  });

  it('names only keys every shipped locale carries', () => {
    const keys = [
      ...(['awaiting_link', 'queued', 'downloading', 'importing', 'completed', 'failed'] as ImportStatus[]).map(
        (status) => importStateCopy(entry(status), STRAVA, 'en').key,
      ),
      ...(['completed', 'failed'] as ImportStatus[]).map(
        (status) => importStateCopy(entry(status, { imported: 1 }), STRAVA, 'en').key,
      ),
      ...[1, 2].flatMap((count) => {
        const { body, action } = importDeleteCopy(count, 'en');
        return [body.key, action.key, importHistoryCopy(entry('completed', { imported: count }), 'en').key];
      }),
      ...IMPORT_SOURCE_COPY.flatMap((copy) => [
        copy.name,
        copy.hint,
        copy.intro,
        copy.step1Title,
        copy.step1Body,
        copy.step2Title,
        copy.progressStarting,
        copy.progress,
        copy.inProgress,
        ...(['link_invalid', 'link_host_not_allowed', 'link_expired', 'link_unreachable'] as const).map((refusal) =>
          linkRefusalKey(refusal, copy),
        ),
        copy.notAnExport,
        ...(copy.unknownColumns ? [copy.unknownColumns] : []),
        ...(copy.intake === 'link'
          ? [copy.step1Open, copy.linkLabel, copy.linkHint, copy.awaitingLink, copy.awaitingLinkLate]
          : [
              copy.pick,
              copy.pickHint,
              copy.resume,
              copy.resumeHint,
              copy.uploading,
              copy.keepOpen,
              copy.awaitingUpload,
              copy.stalled,
              copy.gone,
            ]),
      ]),
      ...[
        'download_failed',
        'link_expired',
        'archive_too_large',
        'archive_unreadable',
        'archive_limit',
        'missing_activities_csv',
        'unknown_locale',
        'spool_missing',
        null,
      ].map((code) => importFailureKey(code, STRAVA)),
      ...[
        'strava_export_error',
        'missing_file',
        'unreadable_file',
        'file_too_large',
        'invalid_row',
        'zero_elapsed',
        'deleted_by_athlete',
      ].map(importOutcomeKey),
      'providers.import.strava.inProgress',
    ];
    for (const [locale, catalogue] of Object.entries(LOCALES)) {
      for (const key of keys) {
        expect(typeof lookup(catalogue, key), `${locale}: ${key}`).toBe('string');
      }
    }
  });
});

describe('the history a provider card counts (carnet#1386)', () => {
  const overview = (imports: ArchiveImport[]) => ({ enabled: true, sources: ['strava_archive', 'garmin_archive'], imports });

  it('counts the completed import of the card\'s own source, while its history is held', () => {
    const imports = [
      entry('completed', { imported: 412 }),
      entry('completed', { source: 'garmin_archive', imported: 87 }),
    ];
    expect(importedHistoryCount(overview(imports), 'strava_archive')).toBe(412);
    expect(importedHistoryCount(overview(imports), 'garmin_archive')).toBe(87);
  });

  it('counts nothing while the import runs or after it stopped', () => {
    for (const status of ['awaiting_link', 'queued', 'downloading', 'importing', 'failed', 'cancelled'] as const) {
      expect(importedHistoryCount(overview([entry(status, { source: 'garmin_archive' })]), 'garmin_archive')).toBe(0);
    }
  });

  it('counts nothing once the athlete deleted the history, for a source with no import, or before the overview is read', () => {
    expect(importedHistoryCount(overview([entry('completed', { history_held: false })]), 'strava_archive')).toBe(0);
    expect(importedHistoryCount(overview([entry('completed')]), 'garmin_archive')).toBe(0);
    expect(importedHistoryCount(undefined, 'strava_archive')).toBe(0);
    expect(importedHistoryCount(null, 'strava_archive')).toBe(0);
  });
});
