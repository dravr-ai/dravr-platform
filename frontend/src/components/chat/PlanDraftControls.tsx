// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's draft on a plan card — the badge, Publish and Discard, a refused publish's conflict and Publish anyway
// ABOUTME: Shown to the thread's owner alone; which state it is in comes from the shared hook, so the phone shows the same

import { useState } from 'react';
import { Check } from 'lucide-react';
import type { DraftState, PlanSubject } from '@pierre/shared-types';
import { PLAN_DRAFT_KEYS, planConflictText } from '@pierre/chat-utils';
import { API_ERROR_KEYS, describeApiError, type PlanDraftActionResult } from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';
import { useAuth } from '../../hooks/useAuth';
import { usePlanDraftCard } from '../../hooks/useCoachPlan';
import { Button, ConfirmDialog, useErrorToast } from '../ui';

interface PlanDraftControlsProps {
  /** The coach's plan thread the card is in. */
  conversationId: string;
  /** The athlete the draft is for. */
  subject: PlanSubject;
  /** The draft the card shows. */
  draft: DraftState;
}

/**
 * The footer of a plan card that shows the coach's draft: a badge saying
 * the athlete cannot see it yet, Discard, and Publish to the athlete. A
 * publish the athlete's own changes refused shows what collided, and Publish
 * anyway; a landed one says the athlete was told. Nothing at all for anyone
 * but the thread's owner, or on a card whose draft has since moved on.
 */
export default function PlanDraftControls({ conversationId, subject, draft }: PlanDraftControlsProps) {
  const { t, language } = useTranslation();
  const { user } = useAuth();
  const showError = useErrorToast();
  const { state, publish, discard, isPublishing, isDiscarding } = usePlanDraftCard(
    conversationId,
    user?.id ?? null,
    draft,
  );
  const [confirmDiscard, setConfirmDiscard] = useState(false);

  if (state.kind === 'hidden') return null;

  const name = subject.display_name.trim() || t('app.unknownMember');
  const busy = isPublishing || isDiscarding;

  /** Say a refusal the card cannot state itself. */
  const report = (result: PlanDraftActionResult) => {
    if (result.kind === 'failed') {
      showError(t('common.error'), describeApiError(result.error, { t, fallbackKey: API_ERROR_KEYS.unknown }));
    }
  };

  if (state.kind === 'published') {
    return (
      <div className="border-t ghost-border-faint px-3 py-2.5" data-testid="plan-draft-controls">
        <p
          role="status"
          className="flex items-center gap-1.5 text-sm text-on-success-container"
          data-testid="plan-draft-published"
        >
          <Check className="h-4 w-4 shrink-0" aria-hidden="true" />
          {t(PLAN_DRAFT_KEYS.published, { name })}
        </p>
      </div>
    );
  }

  if (state.kind === 'consentRevoked') {
    return (
      <div className="border-t ghost-border-faint px-3 py-2.5" data-testid="plan-draft-controls">
        <p
          role="alert"
          className="rounded-lg bg-error/10 px-3 py-2 text-sm text-on-error-container"
          data-testid="plan-draft-consent-revoked"
        >
          {t(PLAN_DRAFT_KEYS.consentRevoked, { name })}
        </p>
      </div>
    );
  }

  const conflict = state.kind === 'conflict' ? planConflictText(state.conflict, name, language, t) : null;

  return (
    <div className="space-y-2.5 border-t ghost-border-faint px-3 py-2.5" data-testid="plan-draft-controls">
      <span className="badge-warning" data-testid="plan-draft-badge">
        {t(PLAN_DRAFT_KEYS.badge, { name })}
      </span>

      {conflict && (
        <div
          role="alert"
          className="space-y-1 rounded-lg bg-warning/10 px-3 py-2 text-on-warning-container"
          data-testid="plan-draft-conflict"
        >
          <p className="text-sm font-semibold">{conflict.title}</p>
          {conflict.lines.map((line) => (
            <p key={line} className="text-sm">
              {line}
            </p>
          ))}
        </div>
      )}

      <div className="flex flex-wrap items-center justify-end gap-2">
        <Button
          variant="tertiary"
          size="sm"
          disabled={busy}
          onClick={() => setConfirmDiscard(true)}
          data-testid="plan-draft-discard"
        >
          {t(PLAN_DRAFT_KEYS.discard)}
        </Button>
        <Button
          variant="primary"
          size="sm"
          loading={isPublishing}
          disabled={busy}
          onClick={() => void publish(conflict !== null).then(report)}
          data-testid={conflict ? 'plan-draft-overwrite' : 'plan-draft-publish'}
        >
          {conflict ? t(PLAN_DRAFT_KEYS.overwrite) : t(PLAN_DRAFT_KEYS.publish, { name })}
        </Button>
      </div>

      <ConfirmDialog
        isOpen={confirmDiscard}
        onClose={() => setConfirmDiscard(false)}
        onConfirm={() =>
          void discard().then((result) => {
            setConfirmDiscard(false);
            report(result);
          })
        }
        title={t(PLAN_DRAFT_KEYS.discard)}
        message={t(PLAN_DRAFT_KEYS.discardConfirm, { name })}
        confirmLabel={t(PLAN_DRAFT_KEYS.discard)}
        variant="danger"
        isLoading={isDiscarding}
      />
    </div>
  );
}
