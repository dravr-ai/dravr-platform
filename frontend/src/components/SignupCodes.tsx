// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Admin console view of signup codes — mint an admin code, list every admin and coach code, revoke
// ABOUTME: A minted code is shown once: only its hash is stored, so the listing carries just its last characters

import { useState, type FormEvent } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { adminApi } from '../services/api';
import {
  SIGNUP_CODES_QUERY_KEY,
  type MintSignupCodeRequest,
  type SignupCodeListing,
} from '../services/api/admin';
import { Button, Badge, Input, Section } from './ui';
import { QUERY_KEYS } from '../constants/queryKeys';
import { formatDate } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';

/** The server's bounds, checked here so a typo is caught before the round trip. */
const PREFIX_PATTERN = /^[A-Za-z0-9]{1,12}$/;
const MAX_USES_LIMIT = 10_000;
const MAX_EXPIRY_DAYS = 365;

/** A whole number in `1..=max`, `undefined` for an empty field, `null` when invalid. */
function parseBounded(raw: string, max: number): number | undefined | null {
  const trimmed = raw.trim();
  if (!trimmed) return undefined;
  if (!/^\d+$/.test(trimmed)) return null;
  const value = Number(trimmed);
  return value >= 1 && value <= max ? value : null;
}

/**
 * A signup code approves whoever enters it at sign-up, or on the waiting
 * screen of a pending account (carnet#1128). An admin mints one here for a
 * cohort; a coach's group invite with `approves_signup` is the other kind, and
 * is listed and revocable here too. Revoking a code stops it working; the
 * accounts it already approved stay approved.
 */
export default function SignupCodes() {
  const { language } = useTranslation();
  const queryClient = useQueryClient();
  const [note, setNote] = useState('');
  const [prefix, setPrefix] = useState('');
  const [maxUses, setMaxUses] = useState('');
  const [expiresInDays, setExpiresInDays] = useState('');
  const [result, setResult] = useState<{ message: string; ok: boolean } | null>(null);
  // The one copy of a freshly minted code. Kept in state, not in the
  // mutation, so a refetch or the next revoke does not take it away.
  const [minted, setMinted] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const {
    data: codes = [],
    isLoading,
    error,
    refetch,
  } = useQuery<SignupCodeListing[]>({
    queryKey: SIGNUP_CODES_QUERY_KEY,
    queryFn: () => adminApi.getSignupCodes(),
  });

  const parsedMaxUses = parseBounded(maxUses, MAX_USES_LIMIT);
  const parsedExpiry = parseBounded(expiresInDays, MAX_EXPIRY_DAYS);
  const prefixInvalid = prefix.trim() !== '' && !PREFIX_PATTERN.test(prefix.trim());
  const formInvalid = prefixInvalid || parsedMaxUses === null || parsedExpiry === null;

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: SIGNUP_CODES_QUERY_KEY });
    // A redemption moves an account out of the pending queue.
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminUsers.pending() });
  };

  const mintMutation = useMutation({
    mutationFn: (request: MintSignupCodeRequest) => adminApi.mintSignupCode(request),
    onSuccess: (data) => {
      setMinted(data.code);
      setCopied(false);
      setResult({ message: data.message, ok: true });
      setNote('');
      setPrefix('');
      setMaxUses('');
      setExpiresInDays('');
      invalidate();
    },
    onError: (err: unknown) => {
      setResult({ message: errorMessage(err, 'Could not mint a signup code'), ok: false });
    },
  });

  const revokeMutation = useMutation({
    mutationFn: (code: SignupCodeListing) => adminApi.revokeSignupCode(code.kind, code.id),
    onSuccess: (data) => {
      setResult({ message: data.message, ok: true });
      invalidate();
    },
    onError: (err: unknown) => {
      setResult({ message: errorMessage(err, 'Could not revoke that signup code'), ok: false });
    },
  });

  const handleSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (formInvalid || mintMutation.isPending) return;
    const request: MintSignupCodeRequest = {};
    if (note.trim()) request.note = note.trim();
    if (prefix.trim()) request.prefix = prefix.trim();
    if (parsedMaxUses !== undefined) request.max_uses = parsedMaxUses;
    if (parsedExpiry !== undefined) request.expires_in_days = parsedExpiry;
    mintMutation.mutate(request);
  };

  const handleCopy = async () => {
    if (!minted) return;
    try {
      await navigator.clipboard.writeText(minted);
      setCopied(true);
    } catch {
      setResult({ message: 'Could not copy to the clipboard — select the code and copy it by hand.', ok: false });
    }
  };

  return (
    <div className="space-y-10">
      <Section
        title="Mint a signup code"
        description="Whoever enters it at sign-up, or on a pending account's waiting screen, is approved without a review. An unconfirmed email activates once its confirmation link is clicked."
      >
        <form onSubmit={handleSubmit} className="space-y-3">
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            <Input
              type="text"
              label="Note"
              placeholder="Cohort, reason"
              value={note}
              onChange={(e) => setNote(e.target.value)}
            />
            <Input
              type="text"
              label="Prefix"
              placeholder="BETA"
              value={prefix}
              onChange={(e) => setPrefix(e.target.value)}
              error={prefixInvalid ? '1 to 12 letters or digits.' : undefined}
            />
            <Input
              type="number"
              min={1}
              max={MAX_USES_LIMIT}
              inputMode="numeric"
              label="Max uses"
              placeholder="Unlimited"
              value={maxUses}
              onChange={(e) => setMaxUses(e.target.value)}
              error={parsedMaxUses === null ? `A whole number from 1 to ${MAX_USES_LIMIT}.` : undefined}
            />
            <Input
              type="number"
              min={1}
              max={MAX_EXPIRY_DAYS}
              inputMode="numeric"
              label="Expires in days"
              placeholder="Never"
              value={expiresInDays}
              onChange={(e) => setExpiresInDays(e.target.value)}
              error={parsedExpiry === null ? `A whole number from 1 to ${MAX_EXPIRY_DAYS}.` : undefined}
            />
          </div>
          <p className="text-xs text-on-surface-variant">Leave a field empty for no limit.</p>
          <div className="flex justify-end">
            <Button type="submit" disabled={formInvalid || mintMutation.isPending}>
              {mintMutation.isPending ? 'Minting…' : 'Mint code'}
            </Button>
          </div>
        </form>

        {minted && (
          <div
            className="mt-4 p-4 rounded-lg bg-surface-container-low border ghost-border space-y-2"
            data-testid="signup-code-minted"
          >
            <p className="text-sm font-medium text-on-surface">
              Copy this code now — you won&apos;t see it again. Only its last characters are kept.
            </p>
            <div className="flex flex-col sm:flex-row sm:items-center gap-3">
              <code className="flex-1 font-mono text-base text-on-surface break-all select-all">
                {minted}
              </code>
              <div className="flex gap-2">
                <Button type="button" size="sm" variant="secondary" onClick={() => void handleCopy()}>
                  {copied ? 'Copied' : 'Copy'}
                </Button>
                <Button type="button" size="sm" variant="outline" onClick={() => setMinted(null)}>
                  Done
                </Button>
              </div>
            </div>
          </div>
        )}

        {result && (
          <p
            role="status"
            className={`mt-3 text-sm ${result.ok ? 'text-on-surface-variant' : 'text-error'}`}
          >
            {result.message}
          </p>
        )}
      </Section>

      <Section
        title={`Signup codes (${codes.length})`}
        description="Admin codes and coaches' group codes. Revoking one stops it working; the accounts it approved stay approved."
        actions={
          <Button onClick={() => void refetch()} variant="outline" size="sm">
            Refresh
          </Button>
        }
      >
        {isLoading ? (
          <div className="space-y-3" aria-busy="true">
            {[...Array(3)].map((_, i) => (
              <div key={i} className="p-4 rounded-lg bg-surface-container-low animate-pulse">
                <div className="h-4 bg-surface-container-high rounded w-48 mb-2"></div>
                <div className="h-3 bg-surface-container-high rounded w-32"></div>
              </div>
            ))}
          </div>
        ) : error ? (
          <div className="p-6 text-center">
            <p className="text-sm font-medium text-on-surface mb-4">Failed to load signup codes</p>
            <Button onClick={() => void refetch()} variant="outline">
              Retry
            </Button>
          </div>
        ) : codes.length === 0 ? (
          <div className="p-6 text-center">
            <p className="text-sm font-medium text-on-surface">No signup codes</p>
            <p className="text-sm text-on-surface-variant">
              Mint one above, or a coach creates one from their group&apos;s invites.
            </p>
          </div>
        ) : (
          <ul className="divide-y divide-outline-variant">
            {codes.map((code) => (
              <li key={`${code.kind}-${code.id}`} className="py-4 flex justify-between items-start">
                <div className="flex-1 min-w-0">
                  <div className="flex flex-wrap items-center gap-2 mb-1">
                    <span className="font-mono font-medium text-on-surface">…{code.code_hint}</span>
                    <Badge variant={code.kind === 'admin' ? 'info' : 'secondary'} className="text-xs">
                      {code.kind}
                    </Badge>
                    <Badge variant={code.live ? 'success' : 'secondary'} className="text-xs">
                      {code.live ? 'live' : 'not live'}
                    </Badge>
                  </div>
                  {code.note && <p className="text-sm text-on-surface-variant mb-1">{code.note}</p>}
                  <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-outline">
                    <span>
                      Uses: {code.use_count} / {code.max_uses ?? '∞'}
                    </span>
                    <span>
                      Expires: {code.expires_at ? formatDate(code.expires_at, language) : 'never'}
                    </span>
                    <span>Minted: {formatDate(code.created_at, language)}</span>
                    <span>By: {code.created_by_email ?? 'unattributed'}</span>
                  </div>
                  {code.redeemed_by.length > 0 && (
                    <ul className="mt-2 space-y-1 text-xs text-on-surface-variant" aria-label="Redeemed by">
                      {code.redeemed_by.map((redeemer) => (
                        <li
                          key={`${redeemer.email}-${redeemer.redeemed_at}`}
                          className="flex flex-wrap items-center gap-2"
                        >
                          <span className="text-on-surface">{redeemer.email}</span>
                          <Badge variant={statusVariant(redeemer.status)} className="text-xs">
                            {redeemer.status}
                          </Badge>
                          <span>redeemed {formatDate(redeemer.redeemed_at, language)}</span>
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
                {code.live && (
                  <Button
                    onClick={() => revokeMutation.mutate(code)}
                    disabled={revokeMutation.isPending}
                    size="sm"
                    variant="outline"
                    className="border-error/50 text-error hover:bg-error/10 ml-4"
                    aria-label={`Revoke code ending ${code.code_hint}`}
                  >
                    Revoke
                  </Button>
                )}
              </li>
            ))}
          </ul>
        )}
      </Section>
    </div>
  );
}

function statusVariant(status: string) {
  switch (status) {
    case 'active':
      return 'success' as const;
    case 'suspended':
      return 'destructive' as const;
    default:
      return 'warning' as const;
  }
}

/** Surface the server's own rejection (a bad prefix, a permission failure) rather than a generic line. */
function errorMessage(err: unknown, fallback: string): string {
  const response = (err as { response?: { data?: { message?: string; error?: string } } })?.response;
  return response?.data?.message ?? response?.data?.error ?? fallback;
}
