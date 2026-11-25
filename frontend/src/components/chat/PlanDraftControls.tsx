// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's draft on a plan card or in the athlete workspace's header — the badge, Publish and Discard, a refused publish's conflict and Publish anyway
// ABOUTME: Shown to the thread's owner alone; which state it is in comes from the shared hook, so the phone shows the same

import { useEffect, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { Check } from 'lucide-react';
import { clsx } from 'clsx';
import type { DraftState, PlanSubject } from '@pierre/shared-types';
import { PLAN_DRAFT_KEYS, planConflictText, planPushComplete, planPushText } from '@pierre/chat-utils';
import {
  API_ERROR_KEYS,
  describeApiError,
  type PlanDraftActionResult,
  type UsePlanDraftCardResult,
} from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';
import { useAuth } from '../../hooks/useAuth';
import { usePlanDraftCard } from '../../hooks/useCoachPlan';
import { cachedConversations } from '../../hooks/useConversationList';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { Badge, Button, ConfirmDialog, useErrorToast, ALERT_TONE } from '../ui';

interface PlanDraftControlsProps {
  /** The coach's plan thread the card is in. */
  conversationId: string;
  /** The athlete the draft is for. */
  subject: PlanSubject;
  /** The draft the card shows. */
  draft: DraftState;
}

interface PlanDraftActionsProps {
  /** The coach's plan thread the draft belongs to. */
  conversationId: string;
  /** The athlete the draft is for. */
  subject: PlanSubject;
  /** The draft's state, Publish and Discard, from `usePlanDraftCard`. */
  card: UsePlanDraftCardResult;
  /**
   * `card`: a plan card's footer, under a hairline, with the badge saying the
   * athlete cannot see the draft yet. `bar`: the athlete workspace's header,
   * where the calendar beside it says what is a draft, so there is no badge
   * and every line lines up at the end of the row.
   */
  layout?: 'card' | 'bar';
  /** The publish button's words in place of "Publish to <name>": the workspace names how many dates change. */
  publishLabel?: string;
  /** Whether Publish may be pressed; false when the draft changes nothing the header can name. Publish anyway always may. */
  canPublish?: boolean;
}

/**
 * A coach's draft controls as they are drawn, from the state the shared hook
 * decides: a badge saying the athlete cannot see the draft yet, Discard, and
 * Publish to the athlete. A publish the athlete's own changes refused shows
 * what collided, and Publish anyway; a landed one says the athlete was told,
 * and whether the plan reached their calendar. Nothing at all for anyone but
 * the thread's owner, or for a draft that has since moved on.
 */
export function PlanDraftActions({
  conversationId,
  subject,
  card,
  layout = 'card',
  publishLabel,
  canPublish = true,
}: PlanDraftActionsProps) {
  const { t, language } = useTranslation();
  const showError = useErrorToast();
  const { state, publish, discard, isPublishing, isDiscarding } = card;
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const bar = layout === 'bar';

  // A publish posts the changed days in the room the plan was drafted from
  // (carnet#1249): that room is read again, so a coach drafting inside it
  // sees the post land under the card.
  const queryClient = useQueryClient();
  const published = state.kind === 'published';
  useEffect(() => {
    if (!published) return;
    const roomId = cachedConversations(queryClient).find((c) => c.id === conversationId)?.subject_group_id;
    if (roomId) void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.room(roomId) });
  }, [published, conversationId, queryClient]);

  if (state.kind === 'hidden') return null;

  const name = subject.display_name.trim() || t('app.unknownMember');
  const busy = isPublishing || isDiscarding;
  const frame = bar ? 'flex max-w-md flex-col items-end gap-2 text-right' : 'border-t ghost-border-faint px-3 py-2.5';

  /** Say a refusal the card cannot state itself. */
  const report = (result: PlanDraftActionResult) => {
    if (result.kind === 'failed') {
      showError(t('common.error'), describeApiError(result.error, { t, fallbackKey: API_ERROR_KEYS.unknown }));
    }
  };

  if (state.kind === 'published') {
    const push = state.calendarPush;
    return (
      <div className={clsx(frame, !bar && 'space-y-1')} data-testid="plan-draft-controls">
        <p
          role="status"
          className="flex items-center gap-1.5 text-sm text-on-success-container"
          data-testid="plan-draft-published"
        >
          <Check className="h-4 w-4 shrink-0" aria-hidden="true" />
          {t(PLAN_DRAFT_KEYS.published, { name })}
        </p>
        {push && (
          <p
            className={`text-sm ${planPushComplete(push) ? 'text-on-success-container' : 'text-on-warning-container'}`}
            data-testid="plan-draft-calendar-push"
          >
            {planPushText(push, name, t)}
          </p>
        )}
      </div>
    );
  }

  if (state.kind === 'consentRevoked') {
    return (
      <div className={frame} data-testid="plan-draft-controls">
        <p
          role="alert"
          className={`${ALERT_TONE.error.tone} rounded-lg px-3 py-2 text-left text-sm`}
          data-testid="plan-draft-consent-revoked"
        >
          {t(PLAN_DRAFT_KEYS.consentRevoked, { name })}
        </p>
      </div>
    );
  }

  const conflict = state.kind === 'conflict' ? planConflictText(state.conflict, name, language, t) : null;

  return (
    <div className={clsx(frame, !bar && 'space-y-2.5')} data-testid="plan-draft-controls">
      {!bar && (
        <Badge variant="warning" data-testid="plan-draft-badge">
          {t(PLAN_DRAFT_KEYS.badge, { name })}
        </Badge>
      )}

      {conflict && (
        <div
          role="alert"
          className={`${ALERT_TONE.warning.tone} space-y-1 rounded-lg px-3 py-2 text-left`}
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
          disabled={busy || (conflict === null && !canPublish)}
          onClick={() => void publish(conflict !== null).then(report)}
          data-testid={conflict ? 'plan-draft-overwrite' : 'plan-draft-publish'}
        >
          {conflict ? t(PLAN_DRAFT_KEYS.overwrite) : (publishLabel ?? t(PLAN_DRAFT_KEYS.publish, { name }))}
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

/**
 * The footer of a plan card that shows the coach's draft: its state from the
 * shared hook, drawn by {@link PlanDraftActions}.
 */
export default function PlanDraftControls({ conversationId, subject, draft }: PlanDraftControlsProps) {
  const { user } = useAuth();
  const card = usePlanDraftCard(conversationId, user?.id ?? null, draft);
  return <PlanDraftActions conversationId={conversationId} subject={subject} card={card} />;
}
