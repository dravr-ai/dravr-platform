// ABOUTME: Import-tombstone refusals shared by web and mobile — reads a 409 or an OAuth return, words the sentence
// ABOUTME: A deleted account's provider account or email waits until the person holding it chooses to start fresh
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { formatDate } from '@pierre/chat-utils';
import { IMPORT_TOMBSTONED_ERROR } from '@pierre/shared-types';
import type { EmailTombstone } from '@pierre/shared-types';
import type { ApiErrorTranslate } from './apiError';

/**
 * A refusal naming an identity a deleted account left behind, as a screen
 * acts on it: the tombstone the clear names, and the date the sentence names.
 */
export interface ImportTombstoneRefusal {
  /** The tombstone `clearImportTombstone` clears. */
  tombstoneId: string;
  /** When the deleted account's data was deleted (RFC 3339). */
  deletedAt: string;
}

/** The error shape read here, duck-typed as `apiError` reads it. */
interface RefusalShape {
  response?: { data?: { details?: { error?: unknown; tombstone_id?: unknown; deleted_at?: unknown } } };
}

/**
 * The tombstone a refused API call names — a 409 whose `details.error` is
 * `import_tombstoned` (an Intervals.icu link) — or `undefined` for any other
 * failure.
 */
export function importTombstoneRefusal(err: unknown): ImportTombstoneRefusal | undefined {
  const details = (err as RefusalShape | null | undefined)?.response?.data?.details;
  if (details?.error !== IMPORT_TOMBSTONED_ERROR) return undefined;
  const { tombstone_id: tombstoneId, deleted_at: deletedAt } = details;
  if (typeof tombstoneId !== 'string' || typeof deletedAt !== 'string') return undefined;
  return { tombstoneId, deletedAt };
}

/**
 * The tombstone a provider-connect return names — its `error` is
 * `import_tombstoned`, with `tombstone_id` and `deleted_at` beside it — or
 * `undefined` for any other return. `get` reads one query parameter, so a
 * `URLSearchParams` and a parsed deep link's params both serve.
 */
export function importTombstoneFromReturn(
  get: (name: string) => unknown
): ImportTombstoneRefusal | undefined {
  if (get('error') !== IMPORT_TOMBSTONED_ERROR) return undefined;
  const tombstoneId = get('tombstone_id');
  const deletedAt = get('deleted_at');
  if (typeof tombstoneId !== 'string' || tombstoneId === '') return undefined;
  if (typeof deletedAt !== 'string' || deletedAt === '') return undefined;
  return { tombstoneId, deletedAt };
}

/** What a sentence needs to word a tombstone: the language and the translator. */
interface TombstoneWording {
  language: string;
  t: ApiErrorTranslate;
}

/**
 * Why a provider account cannot be connected: it belonged to a Dravr account
 * deleted on that date. `providerName` is the brand the card shows.
 */
export function describeProviderTombstone(
  refusal: ImportTombstoneRefusal,
  providerName: string,
  { language, t }: TombstoneWording
): string {
  return t('providers.connectTombstoned', {
    provider: providerName,
    date: formatDate(refusal.deletedAt, language),
  });
}

/** Why a pending account waits: its email belonged to an account deleted on that date. */
export function describeEmailTombstone(tombstone: EmailTombstone, { language, t }: TombstoneWording): string {
  return t('auth.pendingEmailTombstoned', { date: formatDate(tombstone.deleted_at, language) });
}
