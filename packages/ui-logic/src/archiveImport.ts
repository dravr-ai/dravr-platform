// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What both clients say about an import: its one state line, the phase its screen shows, and every code's copy key
// ABOUTME: Pure functions over the wire shape, so the web window and the mobile screen cannot drift apart on a state

import {
  GARMIN_ARCHIVE_SOURCE,
  STRAVA_ARCHIVE_SOURCE,
  isFinalImportStatus,
  linkRefusalOf,
  type ArchiveImport,
  type ImportLinkRefusal,
} from '@pierre/shared-types';
import {
  countKey,
  IMPORT_COMPLETED_KEYS,
  IMPORT_DELETE_CONFIRM_ACTION_KEYS,
  IMPORT_DELETE_CONFIRM_BODY_KEYS,
  IMPORT_HISTORY_LINE_KEYS,
  IMPORT_STOPPED_KEYS,
} from '@pierre/shared-constants';

/** How long an import may wait for its link before its line asks whether the service is late. */
export const AWAITING_LINK_LATE_MS = 3 * 24 * 60 * 60 * 1000;

/** How often a screen asks for an import's progress while it runs. */
export const IMPORT_POLL_INTERVAL_MS = 5000;

/** An i18n key and its values. */
export interface CopyLine {
  key: string;
  values?: Record<string, string | number>;
}

/** A date as the athlete reads it in `locale`. */
export function importDate(iso: string | null, locale: string): string {
  if (!iso) return '';
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return '';
  return date.toLocaleDateString(locale, { year: 'numeric', month: 'short', day: 'numeric' });
}

/** A size in bytes as the athlete reads it ("1.2 GB"). */
export function importSize(bytes: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return `${unit === 0 ? value : value.toFixed(1)} ${units[unit]}`;
}

/**
 * The copy of one source's import that names the service: its row in the
 * source list, where its export is requested, the link its email carries,
 * and the refusals and failures that point back at it. Every other line of
 * an import screen is shared by all sources.
 */
export interface ImportSourceCopy {
  /** The source key the server offers. */
  source: string;
  /** Its name in the source list and as the import's heading. */
  name: string;
  /** The line under its name in the source list. */
  hint: string;
  /** The service's page where the athlete requests the export. */
  exportPage: string;
  intro: string;
  step1Title: string;
  step1Body: string;
  step1Open: string;
  step2Title: string;
  linkLabel: string;
  linkHint: string;
  progressStarting: string;
  inProgress: string;
  /** The source row's line while the import waits for the service's email. */
  awaitingLink: string;
  /** That line once the wait runs past {@link AWAITING_LINK_LATE_MS}. */
  awaitingLinkLate: string;
  /** A link refusal that names the service; `link_unreachable` names none and is shared. */
  refusal: Record<Exclude<ImportLinkRefusal, 'link_unreachable'>, string>;
  /** The `not_an_export` failure. */
  notAnExport: string;
}

const STRAVA_COPY: ImportSourceCopy = {
  source: STRAVA_ARCHIVE_SOURCE,
  name: 'providers.import.sources.strava',
  hint: 'providers.import.sources.stravaHint',
  exportPage: 'https://www.strava.com/athlete/delete_your_account',
  intro: 'providers.import.strava.intro',
  step1Title: 'providers.import.strava.step1Title',
  step1Body: 'providers.import.strava.step1Body',
  step1Open: 'providers.import.strava.step1Open',
  step2Title: 'providers.import.strava.step2Title',
  linkLabel: 'providers.import.strava.linkLabel',
  linkHint: 'providers.import.strava.linkHint',
  progressStarting: 'providers.import.strava.progressStarting',
  inProgress: 'providers.import.strava.inProgress',
  awaitingLink: 'providers.import.state.awaitingLink',
  awaitingLinkLate: 'providers.import.state.awaitingLinkLate',
  refusal: {
    link_invalid: 'providers.import.strava.refusal.link_invalid',
    link_host_not_allowed: 'providers.import.strava.refusal.link_host_not_allowed',
    link_expired: 'providers.import.strava.refusal.link_expired',
  },
  notAnExport: 'providers.import.strava.failure.not_an_export',
};

const GARMIN_COPY: ImportSourceCopy = {
  source: GARMIN_ARCHIVE_SOURCE,
  name: 'providers.import.sources.garmin',
  hint: 'providers.import.sources.garminHint',
  // Garmin's account site, which picks the reader's region: Account, then
  // Data Management, then Export Your Data.
  exportPage: 'https://www.garmin.com/account/datamanagement/exportdata/',
  intro: 'providers.import.garmin.intro',
  step1Title: 'providers.import.garmin.step1Title',
  step1Body: 'providers.import.garmin.step1Body',
  step1Open: 'providers.import.garmin.step1Open',
  step2Title: 'providers.import.garmin.step2Title',
  linkLabel: 'providers.import.garmin.linkLabel',
  linkHint: 'providers.import.garmin.linkHint',
  progressStarting: 'providers.import.garmin.progressStarting',
  inProgress: 'providers.import.garmin.inProgress',
  awaitingLink: 'providers.import.garmin.awaitingLink',
  awaitingLinkLate: 'providers.import.garmin.awaitingLinkLate',
  refusal: {
    link_invalid: 'providers.import.garmin.refusal.link_invalid',
    link_host_not_allowed: 'providers.import.garmin.refusal.link_host_not_allowed',
    link_expired: 'providers.import.garmin.refusal.link_expired',
  },
  notAnExport: 'providers.import.garmin.failure.not_an_export',
};

/** Every source both clients can show, in the order their lists show them. */
export const IMPORT_SOURCE_COPY: readonly ImportSourceCopy[] = [STRAVA_COPY, GARMIN_COPY];

/** The copy of `source`, or `undefined` for a source this build cannot show. */
export function importSourceCopy(source: string): ImportSourceCopy | undefined {
  return IMPORT_SOURCE_COPY.find((copy) => copy.source === source);
}

/**
 * The sources of `offered` this build can show, in the server's order. A
 * source the server offers that this build has no copy for is left out
 * rather than shown under another service's words.
 */
export function offeredImportSources(offered: readonly string[]): ImportSourceCopy[] {
  return offered.flatMap((source) => importSourceCopy(source) ?? []);
}

/** The one line the row of `source` in the source list shows for its import `entry`. */
export function importStateCopy(
  entry: ArchiveImport,
  source: ImportSourceCopy,
  locale: string,
  now = Date.now(),
): CopyLine {
  switch (entry.status) {
    case 'awaiting_link': {
      const waited = now - new Date(entry.created_at).getTime();
      return { key: waited > AWAITING_LINK_LATE_MS ? source.awaitingLinkLate : source.awaitingLink };
    }
    case 'queued':
      return { key: 'providers.import.state.queued' };
    case 'downloading':
      return { key: 'providers.import.state.downloading' };
    case 'importing':
      return {
        key: 'providers.import.state.importing',
        values: { processed: entry.processed_rows, total: entry.total_rows ?? 0 },
      };
    case 'completed':
      if (!entry.history_held) return { key: 'providers.import.strava.deleted' };
      return {
        key: countKey(entry.imported, locale, IMPORT_COMPLETED_KEYS),
        values: { count: entry.imported, date: importDate(entry.finished_at, locale) },
      };
    case 'failed':
    case 'cancelled':
      return { key: countKey(entry.imported, locale, IMPORT_STOPPED_KEYS), values: { count: entry.imported } };
  }
}

/** The import screen's history line for a completed import: how many activities it holds, and when it landed. */
export function importHistoryCopy(entry: ArchiveImport, locale: string): CopyLine {
  return {
    key: countKey(entry.imported, locale, IMPORT_HISTORY_LINE_KEYS),
    values: { count: entry.imported, date: importDate(entry.finished_at, locale) },
  };
}

/** The confirmation before `count` imported activities are deleted: its body and its action's label. */
export function importDeleteCopy(count: number, locale: string): { body: CopyLine; action: CopyLine } {
  return {
    body: { key: countKey(count, locale, IMPORT_DELETE_CONFIRM_BODY_KEYS), values: { count } },
    action: { key: countKey(count, locale, IMPORT_DELETE_CONFIRM_ACTION_KEYS), values: { count } },
  };
}

/** What a source's import screen shows, from its latest import. */
export type ImportPhase =
  /** No import yet, or the last one stopped: ask for the export and its link. */
  | 'request'
  /** Waiting for the athlete to paste the link the service emails. */
  | 'awaiting_link'
  /** Spooling or importing: progress, and a cancel. */
  | 'running'
  /** The last import completed and its history is still held: its summary and that history. */
  | 'completed';

/** The phase an import (or none) puts the screen in. */
export function importPhase(entry: ArchiveImport | null | undefined): ImportPhase {
  if (!entry) return 'request';
  if (entry.status === 'awaiting_link') return 'awaiting_link';
  if (!isFinalImportStatus(entry.status)) return 'running';
  return entry.status === 'completed' && entry.history_held ? 'completed' : 'request';
}

/** The copy key for a link to the export of `source` the server refused. */
export function linkRefusalKey(refusal: ImportLinkRefusal, source: ImportSourceCopy): string {
  if (refusal === 'link_unreachable') return 'providers.import.strava.refusal.link_unreachable';
  return source.refusal[refusal];
}

/** The copy key for the failure code of an import of `source`; an unknown code reads as the generic error. */
export function importFailureKey(code: string | null, source: ImportSourceCopy): string {
  if (code === 'not_an_export') return source.notAnExport;
  const known = [
    'download_failed',
    'link_expired',
    'archive_too_large',
    'archive_unreadable',
    'archive_limit',
    'missing_activities_csv',
    'unknown_locale',
    'spool_missing',
  ];
  return code && known.includes(code) ? `providers.import.strava.failure.${code}` : 'providers.import.strava.genericError';
}

/** The copy key for one report row's outcome; an unknown one reads as unreadable. */
export function importOutcomeKey(outcome: string): string {
  const known = [
    'strava_export_error',
    'missing_file',
    'unreadable_file',
    'file_too_large',
    'invalid_row',
    'zero_elapsed',
    'deleted_by_athlete',
  ];
  return `providers.import.strava.outcome.${known.includes(outcome) ? outcome : 'unreadable_file'}`;
}

/**
 * The copy key for a start of an import of `source` the server refused: the
 * link refusal it names, a second import while one runs (409), or the
 * generic error.
 */
export function startErrorKey(
  status: number | undefined,
  message: string | undefined,
  source: ImportSourceCopy,
): string {
  const refusal = linkRefusalOf(message);
  if (refusal) return linkRefusalKey(refusal, source);
  if (status === 409) return source.inProgress;
  return 'providers.import.strava.genericError';
}

/** The localStorage key an import window writes when an import finishes, for the tab it was opened from. */
export const IMPORT_RESULT_STORAGE_KEY = 'pierre_import_result';
