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
