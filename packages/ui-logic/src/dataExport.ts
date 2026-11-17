// ABOUTME: Reads the end of a downloaded data export — whether the document is whole, and whether the server cut it short
// ABOUTME: Shared by web and mobile, which each read only the file's last bytes: the document can reach half a gigabyte

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/**
 * Enough of the end of the file to hold the document's closing fields, whose
 * longest list names every table once.
 */
export const EXPORT_TAIL_BYTES = 8192;

/**
 * The error a download cut short in transit is thrown with. A code, not a
 * sentence: the athlete reads the section's translated failure, never this.
 */
export const EXPORT_CUT_SHORT = 'export-cut-short';

/**
 * What closes a whole document: `complete`, the limit that stopped it (absent
 * from a server that predates the field), and the two table lists, which end
 * the file. A row's own text cannot end the file like this.
 */
const DOCUMENT_END =
  /"complete":(true|false),(?:"limit_reached":(null|"size"|"time"),)?"failed_tables":\[([^\]]*)\],"skipped_tables":\[([^\]]*)\]\}\s*$/;

/** The limit the server stopped writing the file at. */
export type ExportLimit = 'size' | 'time';

/** What the end of a downloaded export says about the file. */
export interface ExportTail {
  /** Every table was written to its end. */
  complete: boolean;
  /** The limit that stopped the file, when one did. */
  limitReached: ExportLimit | null;
  /** Some table could not be read (`failed_tables` names it). */
  hasFailedTables: boolean;
  /** Some table was not reached (`skipped_tables` names it). */
  hasSkippedTables: boolean;
}

/**
 * The closing fields of a whole document, read from its last bytes; `null`
 * when the text does not end like a document, which is a download cut short.
 */
export function readExportTail(tail: string): ExportTail | null {
  const match = DOCUMENT_END.exec(tail);
  if (!match) return null;
  const limit = match[2];
  return {
    complete: match[1] === 'true',
    limitReached: limit === '"size"' ? 'size' : limit === '"time"' ? 'time' : null,
    hasFailedTables: match[3].trim() !== '',
    hasSkippedTables: match[4].trim() !== '',
  };
}

/** Every sentence a whole but partial file can be saved with. */
export const EXPORT_PARTIAL_KEYS = [
  'dataExport.partial',
  'dataExport.partialLimit',
  'dataExport.partialLimitAndFailed',
  'dataExport.partialSize',
  'dataExport.partialSizeAndFailed',
  'dataExport.partialTime',
  'dataExport.partialTimeAndFailed',
] as const;

/** One of {@link EXPORT_PARTIAL_KEYS}. */
export type ExportPartialKey = (typeof EXPORT_PARTIAL_KEYS)[number];

/**
 * The sentence a whole but partial file is saved with, or `null` for a
 * complete one. The file is still the athlete's: it is saved, and this says
 * what it lacks, from what its end holds and nothing more. The limit names
 * itself when the server wrote `limit_reached`; a server that predates the
 * field leaves only `skipped_tables` to say a limit stopped the file, and not
 * which. A table that could not be read is said beside any limit, never in
 * its place.
 */
export function exportPartialKey(tail: ExportTail): ExportPartialKey | null {
  if (tail.complete) return null;
  const failed = tail.hasFailedTables;
  if (tail.limitReached === 'size') {
    return failed ? 'dataExport.partialSizeAndFailed' : 'dataExport.partialSize';
  }
  if (tail.limitReached === 'time') {
    return failed ? 'dataExport.partialTimeAndFailed' : 'dataExport.partialTime';
  }
  if (tail.hasSkippedTables) {
    return failed ? 'dataExport.partialLimitAndFailed' : 'dataExport.partialLimit';
  }
  return 'dataExport.partial';
}

/** The fewest characters a passphrase sealing the originals bundle has (`MIN_PASSPHRASE_CHARS` server side). */
export const SEAL_MIN_CHARS = 12;

/** The most characters a sealing passphrase has (`MAX_PASSPHRASE_CHARS` server side). */
export const SEAL_MAX_CHARS = 1024;

/** How often a bundle still building is read again. */
export const ORIGINALS_POLL_MS = 5000;

/**
 * Whether `passphrase` can seal a bundle: empty asks for no seal; anything
 * else is counted in characters, as the server counts it, never in bytes.
 */
export function sealPassphraseAccepted(passphrase: string): boolean {
  if (passphrase === '') return true;
  const chars = Array.from(passphrase).length;
  return chars >= SEAL_MIN_CHARS && chars <= SEAL_MAX_CHARS;
}

/** The bundle states a section reads, as the server names them. */
export interface OriginalsBundleState {
  status: 'building' | 'ready' | 'failed';
  missing: number | null;
  damaged: number | null;
}

/** Every sentence the originals bundle section can show. */
export const ORIGINALS_KEYS = [
  'dataExport.originalsBuilding',
  'dataExport.originalsBusy',
  'dataExport.originalsFailed',
  'dataExport.originalsIncomplete',
  'dataExport.originalsOtherBuilding',
  'dataExport.originalsRateLimited',
  'dataExport.originalsReady',
  'dataExport.originalsSealTooShort',
] as const;

/** One of {@link ORIGINALS_KEYS}. */
export type OriginalsKey = (typeof ORIGINALS_KEYS)[number];

/** Whether the section should read the bundle again: it is still being built. */
export function originalsStillBuilding(bundle: OriginalsBundleState | null | undefined): boolean {
  return bundle?.status === 'building';
}

/** The sentence a bundle's state is shown with. */
export function originalsStateKey(bundle: OriginalsBundleState): OriginalsKey {
  if (bundle.status === 'building') return 'dataExport.originalsBuilding';
  if (bundle.status === 'failed') return 'dataExport.originalsFailed';
  return 'dataExport.originalsReady';
}

/**
 * Whether a ready bundle lacks files: some were gone from storage or did not
 * match their digest. Its manifest names them; the section says so.
 */
export function originalsIncomplete(bundle: OriginalsBundleState): boolean {
  return bundle.status === 'ready' && ((bundle.missing ?? 0) > 0 || (bundle.damaged ?? 0) > 0);
}

/**
 * The `error` code an export refusal carries in its body (`export_busy`,
 * `bundle_building`, `too_many_bundles`), never its prose. Not
 * `refusalCode`, which reads an `ErrorCode` refusal's `code`.
 */
export function exportRefusalCode(err: unknown): string | undefined {
  const code = (err as { response?: { data?: { error?: unknown } } } | null | undefined)?.response?.data?.error;
  return typeof code === 'string' ? code : undefined;
}

/**
 * The sentence a refused request for a bundle is shown with, from the
 * answer's status and the `error` code the server sends (never its prose).
 */
export function originalsRefusalKey(status: number | undefined, code: string | undefined): OriginalsKey {
  if (status === 429) return 'dataExport.originalsRateLimited';
  if (status === 503 && code === 'export_busy') return 'dataExport.originalsBusy';
  if (status === 409 && code === 'bundle_building') return 'dataExport.originalsOtherBuilding';
  if (status === 400) return 'dataExport.originalsSealTooShort';
  return 'dataExport.originalsFailed';
}
