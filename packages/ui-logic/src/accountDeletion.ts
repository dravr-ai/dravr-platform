// ABOUTME: Self-serve account deletion rules shared by web and mobile — confirmation matching and refusal wording
// ABOUTME: Reads the server's `error` code and blockers off a refused POST /api/user/account-deletion
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import type {
  AccountDeletionBlocker,
  AccountDeletionBlockerKind,
  AccountDeletionRefusal,
} from '@pierre/shared-types';
import type { ApiErrorTranslate } from './apiError';

/** The refusal codes the server sends, each with the key it reads as. */
const REFUSAL_KEYS: Record<Exclude<AccountDeletionRefusal, 'blocked'>, string> = {
  email_mismatch: 'accountDeletion.emailMismatch',
  password_required: 'accountDeletion.passwordRequired',
  password_incorrect: 'accountDeletion.passwordIncorrect',
  too_many_attempts: 'accountDeletion.tooManyAttempts',
};

const BLOCKER_KINDS: ReadonlySet<AccountDeletionBlockerKind> = new Set([
  'owns_coaching_group',
  'coaches_coaching_group',
  'created_group_invite',
  'admin_config_override',
  'admin_config_audit',
  'tenant_oauth_credentials',
  'llm_credentials',
  'approved_user',
  'owns_tenant',
  'authored_agent',
  'billing_subscription',
]);

/** A refused deletion, as the screen acts on it. */
export interface AccountDeletionFailure {
  /** The sentence to show, already translated. */
  message: string;
  /** What blocks the delete, when the server refused over blockers. */
  blockers: AccountDeletionBlocker[];
}

interface RefusalShape {
  response?: {
    data?: {
      error?: unknown;
      blockers?: unknown;
    };
  };
}

/**
 * Whether the typed confirmation names the account. The server compares the
 * normalized (trimmed, lower-cased) addresses, so the button enables on the
 * same rule.
 */
export function emailConfirms(typed: string, accountEmail: string): boolean {
  const normalized = typed.trim().toLowerCase();
  return normalized.length > 0 && normalized === accountEmail.trim().toLowerCase();
}

function isBlocker(value: unknown): value is AccountDeletionBlocker {
  if (typeof value !== 'object' || value === null) {
    return false;
  }
  const { kind, detail } = value as { kind?: unknown; detail?: unknown };
  return (
    typeof kind === 'string' &&
    BLOCKER_KINDS.has(kind as AccountDeletionBlockerKind) &&
    typeof detail === 'string'
  );
}

/** One blocker as the athlete reads it. */
export function describeAccountDeletionBlocker(
  blocker: AccountDeletionBlocker,
  t: ApiErrorTranslate,
): string {
  return t(`accountDeletion.blocker.${blocker.kind}`, { detail: blocker.detail });
}

/**
 * What a failed deletion tells the athlete: the refusal the server named by
 * its `error` code (with the blockers of a 409), or the generic failure for
 * anything else — a network error, or a disconnect that failed part-way.
 */
export function describeAccountDeletionFailure(
  err: unknown,
  t: ApiErrorTranslate,
): AccountDeletionFailure {
  const data = (err as RefusalShape | null | undefined)?.response?.data;
  const code = data?.error;
  if (code === 'blocked') {
    const blockers = Array.isArray(data?.blockers) ? data.blockers.filter(isBlocker) : [];
    return { message: t('accountDeletion.blockedBody'), blockers };
  }
  if (typeof code === 'string' && code in REFUSAL_KEYS) {
    return { message: t(REFUSAL_KEYS[code as keyof typeof REFUSAL_KEYS]), blockers: [] };
  }
  return { message: t('accountDeletion.failed'), blockers: [] };
}
