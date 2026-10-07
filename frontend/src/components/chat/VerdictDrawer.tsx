// ABOUTME: The claim-verdict drawer — every verdict on one reply for the chat chip, one row for the admin triage table
// ABOUTME: The athlete reads studies and the source reply as pills; the admin keeps the raw ids and layer
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useId, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { clsx } from 'clsx';
import { BookOpen, ExternalLink, MessageCircle, MoreHorizontal } from 'lucide-react';
import type { ClaimVerdict } from '@pierre/shared-types';
import { VERDICT_STATUS_TONE } from '@pierre/shared-types';
import { EVIDENCE_STRENGTH_LABEL_KEY, VERDICT_STATUS_LABEL_KEY } from '@pierre/shared-constants';
import { claimInContext, formatDateTime, parseEvidenceRefs } from '@pierre/chat-utils';
import type { EvidenceRef } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';

/** The reply the verdicts were drawn from, as the chat surface holds it. */
export interface VerdictSource {
  /** What the conversation is called — the thread's stored title. */
  title: string;
  /** The reply's stored content. */
  content: string;
  /** RFC3339 instant the reply was written. */
  createdAt: string;
}

interface VerdictDrawerProps {
  /** The rows to show — every verdict on one reply, or the one row an admin picked. */
  verdicts: ClaimVerdict[];
  /** The rows are still on their way: the chip landed before the read did. */
  loading?: boolean;
  /** Dismiss the drawer. */
  onClose: () => void;
  /**
   * Send a claim back to the agent as a follow-up question. Passed by the
   * chat surface, where the athlete can act on it; omitted by the admin
   * triage table, which is reading someone else's conversation.
   */
  onAskAboutClaim?: (verdict: ClaimVerdict) => void;
  /**
   * Hand support the ids that locate a verdict. Passed by the chat surface,
   * which owns the clipboard and its toast; the card then offers it from an
   * actions menu instead of printing the ids.
   */
  onCopyReference?: (verdict: ClaimVerdict) => void;
  /**
   * The reply the verdicts were drawn from. Passed by the chat surface, which
   * holds the thread; the card names its conversation in a pill whose
   * preview shows the passage the claim sits in.
   */
  source?: VerdictSource;
  /**
   * Extra content under one card. Passed by the admin triage table, which
   * renders its knob and disposition panel here; the chat surface passes
   * nothing. A slot rather than an import, so the operator chrome never
   * reaches the drawer an athlete opens. Its presence also selects the
   * operator card: every chip, and the raw ids under Provenance.
   */
  renderTriage?: (verdict: ClaimVerdict) => ReactNode;
}

/**
 * `training_prescription` reads as "Training Prescription" to a human.
 * LIMITATION(registre#800): `humanizeCategory` prints the English enum in every locale; verdict categories have no label keys.
 */
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

/** One evidence link, printed the way the operator reads it: the raw id. */
function OperatorReference({ reference }: { reference: EvidenceRef }) {
  if (!reference.href) {
    return <li className="font-mono text-xs text-on-surface">{reference.id}</li>;
  }
  return (
    <li className="font-mono text-xs">
      <a href={reference.href} target="_blank" rel="noopener noreferrer" className="text-primary hover:underline">
        {reference.id}
      </a>
    </li>
  );
}

/** Pill classes shared by the study links and the source pill. */
const PILL_CLASS =
  'inline-flex min-h-9 max-w-full items-center gap-2 rounded-full border ghost-border px-3 text-sm font-medium touch-target';

/**
 * The studies behind a verdict, one pill each, named by author and year
 * ("Rønnestad & Mujika, 2014") with the full title on hover. A study the
 * corpus does not name falls back to "Read the study" when it is the only
 * one, and is numbered in stored order otherwise. An id that names no known
 * page keeps its raw text, unlinked, rather than disappearing.
 */
function StudyPills({ references }: { references: EvidenceRef[] }) {
  const { t } = useTranslation();
  return (
    <>
      {references.map((reference, index) => {
        const label =
          reference.label ??
          (references.length === 1 ? t('chat.verdictReadStudy') : t('chat.verdictStudyN', { n: index + 1 }));
        if (!reference.href) {
          return (
            <span key={reference.id} className={clsx(PILL_CLASS, 'font-mono text-xs text-on-surface-variant')}>
              {reference.id}
            </span>
          );
        }
        return (
          <a
            key={reference.id}
            href={reference.href}
            target="_blank"
            rel="noopener noreferrer"
            title={reference.title ?? reference.id}
            data-testid="verdict-study-link"
            className={clsx(PILL_CLASS, 'text-primary hover:bg-surface-container-low')}
          >
            <BookOpen className="h-4 w-4 flex-shrink-0" aria-hidden="true" />
            <span className="min-w-0 truncate">{label}</span>
            <ExternalLink className="h-3 w-3 flex-shrink-0" aria-hidden="true" />
          </a>
        );
      })}
    </>
  );
}

/**
 * The conversation a verdict came from, named in a pill. Hovering it, or
 * reaching it by keyboard, shows the passage of the reply the claim sits in;
 * a press pins that preview open, which is how a touch screen reaches it.
 * "See it in the conversation" closes the drawer — the reply is the one the
 * chip that opened it hangs under.
 */
function SourcePill({
  source,
  claim,
  language,
  onShowReply,
}: {
  source: VerdictSource;
  claim: string;
  language: string;
  onShowReply: () => void;
}) {
  const { t } = useTranslation();
  const [pinned, setPinned] = useState(false);
  const previewId = useId();
  const passage = claimInContext(source.content, claim);

  return (
    // No `relative` here: the preview spans the whole pill row, which is.
    // `min-w-0` lets a long title shrink the pill on a phone instead of
    // pushing it past the drawer's edge.
    <div className="group/source min-w-0 max-w-full">
      <button
        type="button"
        onClick={() => setPinned((open) => !open)}
        aria-expanded={pinned}
        aria-controls={previewId}
        data-testid="verdict-source-pill"
        className={clsx(PILL_CLASS, 'bg-surface-container-low text-on-surface hover:bg-surface-container')}
      >
        <MessageCircle className="h-4 w-4 flex-shrink-0 text-primary" aria-hidden="true" />
        <span className="min-w-0 truncate">{source.title}</span>
      </button>
      {/* `pt-2` rather than a margin, so the pointer can cross into the
          preview without leaving the hover group. Hover reveals it only
          where the device can hover: a touch screen keeps :hover on the
          last element tapped, which would hold the preview open after the
          second tap un-pins it. */}
      <div
        id={previewId}
        data-testid="verdict-source-preview"
        className={clsx(
          'absolute inset-x-0 top-full z-10 pt-2',
          pinned
            ? 'block'
            : 'hidden [@media(hover:hover)]:group-hover/source:block group-has-[:focus-visible]/source:block',
        )}
      >
        <div className="space-y-2 rounded-xl border ghost-border bg-surface-container-lowest p-3 shadow-floating">
          <p className="text-xs text-outline">{formatDateTime(source.createdAt, language)}</p>
          <p className="text-sm text-on-surface">
            {passage.before}
            {passage.match ? (
              <mark className="rounded bg-primary-container px-0.5 text-on-primary-container">{passage.match}</mark>
            ) : null}
            {passage.after}
          </p>
          <button type="button" onClick={onShowReply} className="text-xs font-medium text-primary hover:underline">
            {t('chat.verdictSeeInConversation')}
          </button>
        </div>
      </div>
    </div>
  );
}

/**
 * The card's actions menu. It carries one action — handing support the ids
 * that locate the verdict — which is how the athlete's card offers those ids
 * without printing them.
 */
function VerdictActions({ verdict, onCopyReference }: { verdict: ClaimVerdict; onCopyReference: (verdict: ClaimVerdict) => void }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) setOpen(false);
    };
    // The drawer closes on Escape from `window`; a menu that is open takes
    // the key first and closes alone.
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      event.stopPropagation();
      setOpen(false);
    };
    document.addEventListener('mousedown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('mousedown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [open]);

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={t('chat.verdictActions')}
        title={t('chat.verdictActions')}
        data-testid="verdict-actions-trigger"
        className="flex h-8 w-8 items-center justify-center rounded-lg text-on-surface-variant hover:bg-surface-container hover:text-on-surface touch-target"
      >
        <MoreHorizontal className="h-4 w-4" aria-hidden="true" />
      </button>
      {open ? (
        <div
          role="menu"
          aria-label={t('chat.verdictActions')}
          className="absolute right-0 z-30 mt-1 w-60 rounded-xl border ghost-border bg-surface-container-lowest p-1.5 shadow-floating"
        >
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setOpen(false);
              onCopyReference(verdict);
            }}
            className="w-full rounded-lg px-3 py-2 text-left text-sm text-on-surface transition-colors hover:bg-surface-container-low touch-target"
          >
            {t('chat.verdictCopyReference')}
          </button>
        </div>
      ) : null}
    </div>
  );
}

/** Everything the drawer says about one verdict. */
function VerdictCard({
  verdict,
  language,
  source,
  onClose,
  onAskAboutClaim,
  onCopyReference,
  renderTriage,
}: {
  verdict: ClaimVerdict;
  language: string;
  source?: VerdictSource;
  onClose: () => void;
  onAskAboutClaim?: (verdict: ClaimVerdict) => void;
  onCopyReference?: (verdict: ClaimVerdict) => void;
  renderTriage?: (verdict: ClaimVerdict) => ReactNode;
}) {
  const { t } = useTranslation();
  const references = parseEvidenceRefs(verdict.evidence_refs, verdict.evidence);
  const operator = Boolean(renderTriage);
  const hasProvenance = Boolean(
    verdict.user_id || verdict.agent_id || verdict.conversation_id || verdict.message_id,
  );
  const emittedLabel = formatDateTime(verdict.created_at, language);
  const statusLabel = t(VERDICT_STATUS_LABEL_KEY[verdict.status]);
  const evidenceLabel = t('chat.evidenceLabel', {
    strength: t(EVIDENCE_STRENGTH_LABEL_KEY[verdict.evidence_strength]),
  });
  const confidenceLabel = t('chat.confidenceLabel', { confidence: (verdict.confidence * 100).toFixed(0) });

  return (
    <article data-testid="verdict-card" className="space-y-4 border-b ghost-border px-5 py-5 text-sm last:border-b-0">
      {operator ? (
        <div className="flex flex-wrap gap-2 text-xs">
          <span className={`rounded-full px-2 py-0.5 ${statusToneClass(verdict)}`}>{statusLabel}</span>
          <span className="rounded-full bg-surface-container-high px-2 py-0.5 text-on-surface">
            {humanizeCategory(verdict.category)}
          </span>
          <span className="rounded-full bg-surface-container-high px-2 py-0.5 text-on-surface">{evidenceLabel}</span>
          <span className="rounded-full bg-surface-container-high px-2 py-0.5 text-on-surface">
            {verdict.layer_fired}
          </span>
          <span className="rounded-full bg-surface-container-high px-2 py-0.5 text-on-surface">{confidenceLabel}</span>
        </div>
      ) : (
        <div className="flex items-center gap-2">
          <span className={`rounded-full px-2 py-0.5 text-xs font-medium ${statusToneClass(verdict)}`}>
            {statusLabel}
          </span>
          <span className="min-w-0 flex-1 text-xs text-outline" data-testid="verdict-meta">
            {[humanizeCategory(verdict.category), evidenceLabel, confidenceLabel].join(' · ')}
          </span>
          {onCopyReference ? <VerdictActions verdict={verdict} onCopyReference={onCopyReference} /> : null}
        </div>
      )}

      <section>
        <h4 className={operator ? 'mb-1 text-xs font-semibold text-outline' : 'sr-only'}>{t('chat.theClaim')}</h4>
        <blockquote className="border-l-2 border-primary bg-surface-container-low p-3 text-on-surface">
          {verdict.claim_text}
        </blockquote>
      </section>

      {verdict.explanation ? (
        <section>
          <h4 className={operator ? 'mb-1 text-xs font-semibold text-outline' : 'sr-only'}>
            {t('chat.detectorFindings')}
          </h4>
          <p className="text-on-surface">{verdict.explanation}</p>
        </section>
      ) : null}

      {operator && references.length > 0 ? (
        <section>
          <h4 className="mb-1 text-xs font-semibold text-outline">{t('chat.evidenceReferences')}</h4>
          <ul className="space-y-1">
            {references.map((reference) => (
              <OperatorReference key={reference.id} reference={reference} />
            ))}
          </ul>
        </section>
      ) : null}

      {!operator && (references.length > 0 || source) ? (
        <div className="relative flex flex-wrap gap-2" data-testid="verdict-pills">
          <StudyPills references={references} />
          {source ? (
            <SourcePill source={source} claim={verdict.claim_text} language={language} onShowReply={onClose} />
          ) : null}
        </div>
      ) : null}

      {operator && hasProvenance ? (
        <section>
          <h4 className="mb-1 text-xs font-semibold text-outline">
            {t('chat.provenanceHeading')}
          </h4>
          <dl className="grid grid-cols-[110px_1fr] gap-x-3 gap-y-1 text-xs">
            <Provenance label={t('chat.provenanceUser')} value={verdict.user_id} />
            <Provenance label={t('chat.provenanceAgent')} value={verdict.agent_id} />
            <Provenance label={t('chat.provenanceConversation')} value={verdict.conversation_id} />
            <Provenance label={t('chat.provenanceMessage')} value={verdict.message_id} />
          </dl>
        </section>
      ) : null}

      <p className="text-xs text-outline">
        {t('frag.verdictEmitted')} {emittedLabel}
      </p>

      {renderTriage ? renderTriage(verdict) : null}

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
 * list instead of the call to action. The athlete's card names the reply's
 * conversation and links its studies; it never prints a raw id.
 */
export default function VerdictDrawer({
  verdicts,
  loading = false,
  onClose,
  onAskAboutClaim,
  onCopyReference,
  source,
  renderTriage,
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
        className="h-full w-full max-w-md overflow-y-auto bg-surface-container-lowest text-on-surface shadow-floating pad-safe-top"
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
                  : t('chat.verdictsCount', { count: verdicts.length })}
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
          <VerdictCard
            key={verdict.id}
            verdict={verdict}
            language={language}
            source={source}
            onClose={onClose}
            onAskAboutClaim={onAskAboutClaim}
            onCopyReference={onCopyReference}
            renderTriage={renderTriage}
          />
        ))}
      </div>
    </div>
  );
}
