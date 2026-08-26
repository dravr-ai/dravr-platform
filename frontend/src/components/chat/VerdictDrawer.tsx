// ABOUTME: The one claim-verdict detail drawer — the chat chip and the admin triage table both open it
// ABOUTME: Typed on the shared ClaimVerdict row, so provenance shows only where the read returned it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect } from 'react';
import type { ClaimVerdict } from '@pierre/shared-types';
import { VERDICT_STATUS_TONE } from '@pierre/shared-types';

interface VerdictDrawerProps {
  /** The row to show. */
  verdict: ClaimVerdict;
  /** Dismiss the drawer. */
  onClose: () => void;
  /**
   * Send the claim back to the coach as a follow-up question. Passed by the
   * chat surface, where the athlete can act on it; omitted by the admin
   * triage table, which is reading someone else's conversation.
   */
  onAskAboutClaim?: () => void;
}

/** `training_prescription` reads as "Training Prescription" to a human. */
function humanizeCategory(category: string): string {
  return category.replace(/_/g, ' ').replace(/\b\w/g, (c: string) => c.toUpperCase());
}

/** The emitted instant in the reader's own timezone, or the raw value. */
function formatTimestamp(iso: string): string {
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

/** Chip classes for the tone the shared rollup assigns a status. */
function statusToneClass(verdict: ClaimVerdict): string {
  switch (VERDICT_STATUS_TONE[verdict.status]) {
    case 'success':
      return 'bg-success/15 text-success';
    case 'warning':
      return 'bg-warning/15 text-warning';
    case 'error':
      return 'bg-error/15 text-error';
    case 'info':
      return 'bg-info/15 text-info';
    case 'secondary':
    default:
      return 'bg-surface-container-high/15 text-on-surface';
  }
}

/** One provenance row, rendered only for an id the read actually returned. */
function Provenance({ label, value }: { label: string; value: string | null | undefined }) {
  if (!value) return null;
  return (
    <>
      <dt className="text-outline">{label}</dt>
      <dd className="font-mono text-on-surface break-all">{value}</dd>
    </>
  );
}

/**
 * Full detail for one flagged claim.
 *
 * There was one of these per caller — a chat drawer and an admin drawer, each
 * with its own copy of `humanizeCategory` and `formatTimestamp` over its own
 * shape for the same database row. This is that drawer, once: the chat surface
 * passes `onAskAboutClaim` and gets the call to action, the admin read carries
 * `user_id`/`tenant_id` and gets the provenance list.
 */
export default function VerdictDrawer({
  verdict,
  onClose,
  onAskAboutClaim,
}: VerdictDrawerProps) {
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  const references = (verdict.evidence_refs ?? '')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);

  const hasProvenance = Boolean(
    verdict.user_id || verdict.coach_id || verdict.conversation_id || verdict.message_id,
  );

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-end bg-black/60"
      role="dialog"
      aria-modal="true"
      aria-label="Claim verdict details"
      onClick={onClose}
    >
      <div
        className="h-full w-full max-w-md overflow-y-auto bg-surface-container-lowest shadow-xl text-on-surface"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="sticky top-0 border-b ghost-border bg-surface-container-lowest/90 px-5 py-4 backdrop-blur">
          <div className="flex items-start justify-between">
            <div>
              <h3 className="text-lg font-semibold text-on-surface">About this claim</h3>
              <p className="mt-0.5 text-xs text-outline">{verdict.id}</p>
            </div>
            <button
              type="button"
              onClick={onClose}
              className="rounded p-1 text-on-surface-variant hover:bg-surface-container hover:text-on-surface"
              aria-label="Close"
            >
              <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M6 18L18 6M6 6l12 12"
                />
              </svg>
            </button>
          </div>
          <div className="mt-3 flex flex-wrap gap-2 text-xs">
            <span className={`rounded-full px-2 py-0.5 ${statusToneClass(verdict)}`}>
              {verdict.status}
            </span>
            <span className="rounded-full bg-surface-container-high px-2 py-0.5 text-on-surface">
              {humanizeCategory(verdict.category)}
            </span>
            <span className="rounded-full bg-surface-container-high px-2 py-0.5 text-on-surface">
              evidence: {verdict.evidence_strength}
            </span>
            <span className="rounded-full bg-surface-container-high px-2 py-0.5 text-on-surface">
              {verdict.layer_fired}
            </span>
            <span className="rounded-full bg-surface-container-high px-2 py-0.5 text-on-surface">
              confidence: {(verdict.confidence * 100).toFixed(0)}%
            </span>
          </div>
        </div>

        <div className="space-y-5 px-5 py-5 text-sm">
          <section>
            <h4 className="mb-1 text-xs font-semibold uppercase tracking-wide text-outline">
              The claim
            </h4>
            <blockquote className="border-l-2 border-primary bg-surface-container-low p-3 text-on-surface">
              {verdict.claim_text}
            </blockquote>
          </section>

          {verdict.explanation ? (
            <section>
              <h4 className="mb-1 text-xs font-semibold uppercase tracking-wide text-outline">
                What the detector found
              </h4>
              <p className="text-on-surface">{verdict.explanation}</p>
            </section>
          ) : null}

          {references.length > 0 ? (
            <section>
              <h4 className="mb-1 text-xs font-semibold uppercase tracking-wide text-outline">
                Evidence references
              </h4>
              <ul className="space-y-1">
                {references.map((ref) => (
                  <li key={ref} className="font-mono text-xs text-on-surface">
                    {ref}
                  </li>
                ))}
              </ul>
            </section>
          ) : null}

          {hasProvenance ? (
            <section>
              <h4 className="mb-1 text-xs font-semibold uppercase tracking-wide text-outline">
                Provenance
              </h4>
              <dl className="grid grid-cols-[110px_1fr] gap-x-3 gap-y-1 text-xs">
                <Provenance label="User" value={verdict.user_id} />
                <Provenance label="Coach" value={verdict.coach_id} />
                <Provenance label="Conversation" value={verdict.conversation_id} />
                <Provenance label="Message" value={verdict.message_id} />
              </dl>
            </section>
          ) : null}

          <section className="text-xs text-outline">
            Verdict emitted {formatTimestamp(verdict.created_at)}
          </section>

          {onAskAboutClaim ? (
            <button
              type="button"
              onClick={onAskAboutClaim}
              className="w-full rounded-lg bg-primary px-4 py-2 text-sm font-medium text-on-primary hover:bg-primary/90"
            >
              Ask me about this claim
            </button>
          ) : null}
        </div>
      </div>
    </div>
  );
}
