// ABOUTME: The claim-verdict drawer — every verdict on one reply for the chat chip, one row for the admin triage table
// ABOUTME: Typed on the shared ClaimVerdict row, so provenance shows only where the read returned it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect } from 'react';
import type { ClaimVerdict } from '@pierre/shared-types';
import { VERDICT_STATUS_TONE } from '@pierre/shared-types';
import { EVIDENCE_STRENGTH_LABEL_KEY, VERDICT_STATUS_LABEL_KEY } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';

interface VerdictDrawerProps {
  /** The rows to show — every verdict on one reply, or the one row an admin picked. */
  verdicts: ClaimVerdict[];
  /** The rows are still on their way: the chip landed before the read did. */
  loading?: boolean;
  /** Dismiss the drawer. */
  onClose: () => void;
  /**
   * Send a claim back to the coach as a follow-up question. Passed by the
   * chat surface, where the athlete can act on it; omitted by the admin
   * triage table, which is reading someone else's conversation.
   */
  onAskAboutClaim?: (verdict: ClaimVerdict) => void;
}

/** `training_prescription` reads as "Training Prescription" to a human. */
function humanizeCategory(category: string): string {
  return category.replace(/_/g, ' ').replace(/\b\w/g, (c: string) => c.toUpperCase());
}

/** Chip classes for the tone the shared rollup assigns a status. */
function statusToneClass(verdict: ClaimVerdict): string {
  switch (VERDICT_STATUS_TONE[verdict.status]) {
    case 'success':
      return 'bg-success/15 text-on-success-container';
    case 'warning':
      return 'bg-warning/15 text-on-warning-container';
    case 'error':
      return 'bg-error/15 text-error';
    case 'info':
      return 'bg-info/15 text-on-info-container';
    case 'secondary':
    default:
      return 'bg-surface-container-high text-on-surface';
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

/** Everything the drawer says about one verdict. */
function VerdictCard({
  verdict,
  language,
  onAskAboutClaim,
}: {
  verdict: ClaimVerdict;
  language: string;
  onAskAboutClaim?: (verdict: ClaimVerdict) => void;
}) {
  const { t } = useTranslation();
  const references = (verdict.evidence_refs ?? '')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
  const hasProvenance = Boolean(
    verdict.user_id || verdict.coach_id || verdict.conversation_id || verdict.message_id,
  );
  const emitted = new Date(verdict.created_at);
  const emittedLabel = Number.isNaN(emitted.getTime())
    ? verdict.created_at
    : new Intl.DateTimeFormat(language, { dateStyle: 'medium', timeStyle: 'short' }).format(emitted);

  return (
    <article data-testid="verdict-card" className="space-y-4 border-b ghost-border px-5 py-5 text-sm last:border-b-0">
      <div className="flex flex-wrap gap-2 text-xs">
        <span className={`rounded-full px-2 py-0.5 ${statusToneClass(verdict)}`}>
          {t(VERDICT_STATUS_LABEL_KEY[verdict.status])}
        </span>
        <span className="rounded-full bg-surface-container-high px-2 py-0.5 text-on-surface">
          {humanizeCategory(verdict.category)}
        </span>
        <span className="rounded-full bg-surface-container-high px-2 py-0.5 text-on-surface">
          {t('chat.evidenceLabel', {
            strength: t(EVIDENCE_STRENGTH_LABEL_KEY[verdict.evidence_strength]),
          })}
        </span>
        <span className="rounded-full bg-surface-container-high px-2 py-0.5 text-on-surface">
          {verdict.layer_fired}
        </span>
        <span className="rounded-full bg-surface-container-high px-2 py-0.5 text-on-surface">
          {t('chat.confidenceLabel', { confidence: (verdict.confidence * 100).toFixed(0) })}
        </span>
      </div>

      <section>
        <h4 className="mb-1 text-xs font-semibold uppercase tracking-wide text-outline">
          {t('chat.theClaim')}
        </h4>
        <blockquote className="border-l-2 border-primary bg-surface-container-low p-3 text-on-surface">
          {verdict.claim_text}
        </blockquote>
      </section>

      {verdict.explanation ? (
        <section>
          <h4 className="mb-1 text-xs font-semibold uppercase tracking-wide text-outline">
            {t('chat.detectorFindings')}
          </h4>
          <p className="text-on-surface">{verdict.explanation}</p>
        </section>
      ) : null}

      {references.length > 0 ? (
        <section>
          <h4 className="mb-1 text-xs font-semibold uppercase tracking-wide text-outline">
            {t('chat.evidenceReferences')}
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
            {t('chat.provenanceHeading')}
          </h4>
          <dl className="grid grid-cols-[110px_1fr] gap-x-3 gap-y-1 text-xs">
            <Provenance label={t('chat.provenanceUser')} value={verdict.user_id} />
            <Provenance label={t('chat.provenanceCoach')} value={verdict.coach_id} />
            <Provenance label={t('chat.provenanceConversation')} value={verdict.conversation_id} />
            <Provenance label={t('chat.provenanceMessage')} value={verdict.message_id} />
          </dl>
        </section>
      ) : null}

      <p className="text-xs text-outline">
        {t('frag.verdictEmitted')} {emittedLabel}
      </p>

      {onAskAboutClaim ? (
        <button
          type="button"
          onClick={() => onAskAboutClaim(verdict)}
          className="w-full rounded-lg bg-primary px-4 py-2 text-sm font-medium text-on-primary hover:bg-primary/90"
        >
          {t('chat.askAboutClaim')}
        </button>
      ) : null}
    </article>
  );
}

/**
 * Every verdict on one reply, in one drawer.
 *
 * The chat chip opens it with all the rows of its message — a reply that
 * drew two chips shows two cards, not the first one twice. The admin triage
 * table opens it with the single row it picked, and gets the provenance
 * list instead of the call to action.
 */
export default function VerdictDrawer({
  verdicts,
  loading = false,
  onClose,
  onAskAboutClaim,
}: VerdictDrawerProps) {
  const { t, language } = useTranslation();
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-end bg-scrim/60"
      role="dialog"
      aria-modal="true"
      aria-label={t('chat.verdictDrawerAria')}
      onClick={onClose}
    >
      <div
        className="h-full w-full max-w-md overflow-y-auto bg-surface-container-lowest text-on-surface shadow-floating"
        onClick={(e) => e.stopPropagation()}
        data-testid="verdict-drawer"
      >
        <div className="sticky top-0 flex items-start justify-between border-b ghost-border bg-surface-container-lowest px-5 py-4">
          <div>
            <h3 className="text-lg font-semibold text-on-surface">
              {verdicts.length === 1 ? t('chat.aboutThisClaim') : t('chat.verdictsTitle')}
            </h3>
            <p className="mt-0.5 text-xs text-outline">
              {loading
                ? t('chat.verdictsLoading')
                : verdicts.length === 1
                  ? t('chat.verdictChipOne', { count: 1, qualifier: t(VERDICT_STATUS_LABEL_KEY[verdicts[0].status]) })
                  : t('chat.verdictChipN', { count: verdicts.length, qualifier: '' }).replace(/\s*·\s*$/, '')}
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded p-1 text-on-surface-variant hover:bg-surface-container hover:text-on-surface"
            aria-label={t('chat.close')}
          >
            <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
        {loading && verdicts.length === 0 ? (
          <div className="flex items-center gap-2 px-5 py-6 text-sm text-on-surface-variant">
            <div className="pierre-spinner h-4 w-4" />
            <span>{t('chat.verdictsLoading')}</span>
          </div>
        ) : null}
        {verdicts.map((verdict) => (
          <VerdictCard key={verdict.id} verdict={verdict} language={language} onAskAboutClaim={onAskAboutClaim} />
        ))}
      </div>
    </div>
  );
}
