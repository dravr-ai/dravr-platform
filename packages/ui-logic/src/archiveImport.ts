// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What both clients say about an import: its one state line, the phase its screen shows, and every code's copy key
// ABOUTME: Pure functions over the wire shape, so the web window and the mobile screen cannot drift apart on a state

import {
  isFinalImportStatus,
  linkRefusalOf,
  type ArchiveImport,
  type ImportLinkRefusal,
} from '@pierre/shared-types';

/** How long an import may wait for its link before its line asks whether Strava is late. */
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

/** The one line a source's row in the source list shows for its import. */
export function importStateCopy(entry: ArchiveImport, locale: string, now = Date.now()): CopyLine {
  switch (entry.status) {
    case 'awaiting_link': {
      const waited = now - new Date(entry.created_at).getTime();
      return {
        key: waited > AWAITING_LINK_LATE_MS ? 'providers.import.state.awaitingLinkLate' : 'providers.import.state.awaitingLink',
      };
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
        key: 'providers.import.state.completed',
        values: { count: entry.imported, date: importDate(entry.finished_at, locale) },
      };
    case 'failed':
    case 'cancelled':
      return { key: 'providers.import.state.stopped', values: { count: entry.imported } };
  }
}

/** What the Strava import screen shows, from its latest import. */
export type ImportPhase =
  /** No import yet, or the last one stopped: ask for the export and its link. */
  | 'request'
  /** Waiting for the athlete to paste the link Strava emails. */
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

/** The copy key for a link refusal. */
export function linkRefusalKey(refusal: ImportLinkRefusal): string {
  return `providers.import.strava.refusal.${refusal}`;
}

/** The copy key for an import's failure code; an unknown code reads as the generic error. */
export function importFailureKey(code: string | null): string {
  const known = [
    'download_failed',
    'link_expired',
    'archive_too_large',
    'archive_unreadable',
    'archive_limit',
    'missing_activities_csv',
    'unknown_locale',
    'spool_missing',
    'not_an_export',
  ];
  return code && known.includes(code) ? `providers.import.strava.failure.${code}` : 'providers.import.strava.genericError';
}

/** The copy key for one report row's outcome; an unknown one reads as unreadable. */
export function importOutcomeKey(outcome: string): string {
  const known = ['strava_export_error', 'missing_file', 'unreadable_file', 'file_too_large', 'invalid_row', 'zero_elapsed'];
  return `providers.import.strava.outcome.${known.includes(outcome) ? outcome : 'unreadable_file'}`;
}

/**
 * The copy key for a start the server refused: the link refusal it names, a
 * second import while one runs (409), or the generic error.
 */
export function startErrorKey(status: number | undefined, message: string | undefined): string {
  const refusal = linkRefusalOf(message);
  if (refusal) return linkRefusalKey(refusal);
  if (status === 409) return 'providers.import.strava.inProgress';
  return 'providers.import.strava.genericError';
}

/** The localStorage key an import window writes when an import finishes, for the tab it was opened from. */
export const IMPORT_RESULT_STORAGE_KEY = 'pierre_import_result';
