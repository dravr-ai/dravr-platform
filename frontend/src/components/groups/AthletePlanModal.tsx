// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coached athlete's live plan, read-only, opened from a Group info member row (carnet#1244)
// ABOUTME: The plan the athlete sees, drawn by the plan card; "Work on <name>'s plan" opens the thread where it changes

import { ClipboardList } from 'lucide-react';
import type { GroupMember } from '@pierre/shared-types';
import { PLAN_THREAD_KEYS, PLAN_VIEW_KEYS } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { useAthletePlan } from '../../hooks/useCoachPlan';
import { Button, Modal } from '../ui';
import WorkoutPlanCard from '../chat/WorkoutPlanCard';

interface AthletePlanModalProps {
  /** The athlete whose plan is open; `null` keeps the modal closed. */
  member: GroupMember | null;
  /** The group the caller coaches them in. */
  groupId: string;
  onClose: () => void;
  /**
   * Open the caller's plan thread for the athlete, where the plan changes.
   * Without it the modal only reads.
   */
  onWorkOnPlan?: (member: GroupMember) => void;
  /** The plan thread is being opened. */
  isOpeningThread?: boolean;
}

/**
 * The plan a coached athlete sees, as their coach reads it outside the plan
 * thread: no draft, nothing editable. An athlete with no published plan, one
 * who stopped sharing with the coach, and a read that failed each say so.
 */
export default function AthletePlanModal({
  member,
  groupId,
  onClose,
  onWorkOnPlan,
  isOpeningThread = false,
}: AthletePlanModalProps) {
  const { t } = useTranslation();
  const { read, isLoading, isError, refetch } = useAthletePlan(
    member?.user_id ?? null,
    member ? groupId : null,
  );
  const name =
    read?.status === 'ok'
      ? read.plan.athlete.display_name
      : (member?.display_name ?? t('groups.memberFallback'));
  const canWorkOnPlan = onWorkOnPlan !== undefined && member !== null && read?.status !== 'consentRevoked';

  let body: React.ReactNode;
  if (isLoading) {
    body = (
      <div className="flex justify-center py-10">
        <div className="pierre-spinner" />
      </div>
    );
  } else if (isError) {
    body = (
      <div className="space-y-3 py-6 text-center" data-testid="athlete-plan-error">
        <p className="text-sm text-on-surface-variant">{t(PLAN_VIEW_KEYS.loadError)}</p>
        <Button variant="secondary" size="sm" onClick={refetch}>
          {t('common.retry')}
        </Button>
      </div>
    );
  } else if (read?.status === 'consentRevoked') {
    body = (
      <p className="py-6 text-center text-sm text-on-surface-variant" data-testid="athlete-plan-revoked">
        {t(PLAN_VIEW_KEYS.consentRevoked, { name })}
      </p>
    );
  } else if (read?.status === 'ok' && read.plan.live === null) {
    body = (
      <p className="py-6 text-center text-sm text-on-surface-variant" data-testid="athlete-plan-empty">
        {t(PLAN_VIEW_KEYS.empty, { name })}
      </p>
    );
  } else if (read?.status === 'ok' && read.plan.live !== null) {
    body = (
      <div className="space-y-3" data-testid="athlete-plan">
        <p className="text-xs text-outline">{t(PLAN_VIEW_KEYS.readOnly, { name })}</p>
        <WorkoutPlanCard plan={read.plan.live} />
      </div>
    );
  }

  return (
    <Modal
      isOpen={member !== null}
      onClose={onClose}
      title={t(PLAN_VIEW_KEYS.title, { name })}
      size="2xl"
      footer={
        canWorkOnPlan ? (
          <div className="flex justify-end">
            <Button
              variant="secondary"
              size="sm"
              onClick={() => onWorkOnPlan(member)}
              loading={isOpeningThread}
              data-testid="athlete-plan-work-on"
            >
              <ClipboardList className="mr-1.5 h-4 w-4" aria-hidden="true" />
              {t(PLAN_THREAD_KEYS.open, { name })}
            </Button>
          </div>
        ) : undefined
      }
    >
      {body}
    </Modal>
  );
}
