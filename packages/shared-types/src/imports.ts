// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The import-your-own-data wire shapes — an import's state, a link check, a report page — and their parsers
// ABOUTME: One declaration both clients read, so the web window and the mobile screen agree on every state and code

/** The source key a Strava export is imported under (`STRAVA_ARCHIVE` in pierre-core). */
export const STRAVA_ARCHIVE_SOURCE = 'strava_archive';

/** Where an import is. The last three are final. */
export type ImportStatus =
  | 'awaiting_link'
  | 'queued'
  | 'downloading'
  | 'importing'
  | 'completed'
  | 'failed'
  | 'cancelled';

const IMPORT_STATUSES: readonly ImportStatus[] = [
  'awaiting_link',
  'queued',
  'downloading',
  'importing',
  'completed',
  'failed',
  'cancelled',
];

/** Whether an import in `status` can no longer change. */
export function isFinalImportStatus(status: ImportStatus): boolean {
  return status === 'completed' || status === 'failed' || status === 'cancelled';
}

/** Why an import failed, as the server's stable codes name it. */
export type ImportFailureCode =
  | 'download_failed'
  | 'link_expired'
  | 'archive_too_large'
  | 'archive_unreadable'
  | 'archive_limit'
  | 'missing_activities_csv'
  | 'unknown_locale'
  | 'spool_missing'
  | 'not_an_export';

/** Why a pasted link was refused before any import started. */
export type ImportLinkRefusal =
  | 'link_invalid'
  | 'link_host_not_allowed'
  | 'link_expired'
  | 'link_unreachable';

const LINK_REFUSALS: readonly ImportLinkRefusal[] = [
  'link_invalid',
  'link_host_not_allowed',
  'link_expired',
  'link_unreachable',
];

/** One import, as the athlete's screens read it. The link never comes back. */
export interface ArchiveImport {
  id: string;
  /** The source it imports, e.g. {@link STRAVA_ARCHIVE_SOURCE}. */
  source: string;
  status: ImportStatus;
  /** Rows in the export's activity list, once downloaded. */
  total_rows: number | null;
  /** Rows read so far. */
  processed_rows: number;
  /** Activities stored or changed. */
  imported: number;
  /** Rows already held unchanged, or skipped by rule. */
  skipped: number;
  /** Rows that could not be imported; the report lists them. */
  failed: number;
  /** Why it failed, when it did; a code this build may not know reads as text. */
  error_code: string | null;
  earliest_start: string | null;
  latest_start: string | null;
  created_at: string;
  updated_at: string;
  finished_at: string | null;
  /** When an import waiting for its link is cancelled. */
  expires_at: string | null;
  /** Whether the athlete still holds the history it brought in: false once they deleted it. */
  history_held: boolean;
}

/** `GET /api/imports`. */
export interface ImportsOverview {
  /** Whether imports run on this deployment. */
  enabled: boolean;
  /** The sources an athlete may import. */
  sources: string[];
  /** Each source's running or latest import. */
  imports: ArchiveImport[];
}

/** `POST /api/imports/{source}/check`. */
export interface ImportLinkCheck {
  accepted: boolean;
  refusal: ImportLinkRefusal | null;
  size_bytes: number | null;
}

/** One row of an import's report: a row of the export that was not imported. */
export interface ImportIssue {
  /** Its line in the export's activity list. */
  row: number;
  activity_id: string | null;
  /** Why, as a stable code (`missing_file`, `unreadable_file`, …). */
  outcome: string;
  detail: string | null;
}

/** `GET /api/imports/{source}/{id}/report`. */
export interface ImportReportPage {
  issues: ImportIssue[];
  next_offset: number | null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isCount(value: unknown): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0;
}

function isNullableString(value: unknown): value is string | null {
  return value === null || typeof value === 'string';
}

function isStatus(value: unknown): value is ImportStatus {
  return typeof value === 'string' && (IMPORT_STATUSES as readonly string[]).includes(value);
}

/** Read one import, or `null` for any other shape. */
export function parseArchiveImport(value: unknown): ArchiveImport | null {
  if (
    !isRecord(value) ||
    typeof value.id !== 'string' ||
    value.id === '' ||
    typeof value.source !== 'string' ||
    !isStatus(value.status) ||
    !(value.total_rows === null || isCount(value.total_rows)) ||
    !isCount(value.processed_rows) ||
    !isCount(value.imported) ||
    !isCount(value.skipped) ||
    !isCount(value.failed) ||
    !isNullableString(value.error_code) ||
    !isNullableString(value.earliest_start) ||
    !isNullableString(value.latest_start) ||
    typeof value.created_at !== 'string' ||
    typeof value.updated_at !== 'string' ||
    !isNullableString(value.finished_at) ||
    !isNullableString(value.expires_at) ||
    typeof value.history_held !== 'boolean'
  ) {
    return null;
  }
  return {
    id: value.id,
    source: value.source,
    status: value.status,
    total_rows: value.total_rows,
    processed_rows: value.processed_rows,
    imported: value.imported,
    skipped: value.skipped,
    failed: value.failed,
    error_code: value.error_code,
    earliest_start: value.earliest_start,
    latest_start: value.latest_start,
    created_at: value.created_at,
    updated_at: value.updated_at,
    finished_at: value.finished_at,
    expires_at: value.expires_at,
    history_held: value.history_held,
  };
}

/** Read a `GET /api/imports` body, or `null` when any import in it is malformed. */
export function parseImportsOverview(body: unknown): ImportsOverview | null {
  if (
    !isRecord(body) ||
    typeof body.enabled !== 'boolean' ||
    !Array.isArray(body.sources) ||
    !body.sources.every((source) => typeof source === 'string') ||
    !Array.isArray(body.imports)
  ) {
    return null;
  }
  const imports: ArchiveImport[] = [];
  for (const entry of body.imports) {
    const parsed = parseArchiveImport(entry);
    if (parsed === null) return null;
    imports.push(parsed);
  }
  return { enabled: body.enabled, sources: body.sources as string[], imports };
}

/** Read a `GET /api/imports/{source}/current` body: the import, `null` when none, or `undefined` for a malformed body. */
export function parseCurrentImport(body: unknown): ArchiveImport | null | undefined {
  if (!isRecord(body) || !('import' in body)) return undefined;
  if (body.import === null) return null;
  return parseArchiveImport(body.import) ?? undefined;
}

/** Read a link check, or `null` for any other shape. */
export function parseImportLinkCheck(body: unknown): ImportLinkCheck | null {
  if (!isRecord(body) || typeof body.accepted !== 'boolean') return null;
  const refusal = body.refusal;
  if (!(refusal === null || (typeof refusal === 'string' && (LINK_REFUSALS as readonly string[]).includes(refusal)))) {
    return null;
  }
  if (!(body.size_bytes === null || isCount(body.size_bytes))) return null;
  return {
    accepted: body.accepted,
    refusal: refusal as ImportLinkRefusal | null,
    size_bytes: body.size_bytes,
  };
}

/** Read a report page, or `null` for any other shape. */
export function parseImportReportPage(body: unknown): ImportReportPage | null {
  if (!isRecord(body) || !Array.isArray(body.issues)) return null;
  if (!(body.next_offset === null || isCount(body.next_offset))) return null;
  const issues: ImportIssue[] = [];
  for (const entry of body.issues) {
    if (
      !isRecord(entry) ||
      !isCount(entry.row) ||
      !isNullableString(entry.activity_id) ||
      typeof entry.outcome !== 'string' ||
      !isNullableString(entry.detail)
    ) {
      return null;
    }
    issues.push({
      row: entry.row,
      activity_id: entry.activity_id,
      outcome: entry.outcome,
      detail: entry.detail,
    });
  }
  return { issues, next_offset: body.next_offset };
}

/** The link refusal code a refused start names at the head of its message, if any. */
export function linkRefusalOf(message: string | undefined): ImportLinkRefusal | null {
  if (!message) return null;
  return LINK_REFUSALS.find((code) => message.includes(code)) ?? null;
}
