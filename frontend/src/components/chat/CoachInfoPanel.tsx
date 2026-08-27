// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coach info for the open thread — title, @handle, category, description and the two coach actions
// ABOUTME: Both actions are commands: removing sends /coach remove, editing opens the coach's Discover sheet

import { AtSign, Pencil, UserMinus } from 'lucide-react';
import { COMMAND_DRAFTS } from '@pierre/shared-constants';
import type { Coach } from '@pierre/shared-types';
import { Button } from '../ui';
import { useTranslation } from '@pierre/i18n';

interface CoachInfoPanelProps {
  /** The coach bound to the conversation. */
  coach: Coach;
  /** Send a turn in the open conversation — how `/coach remove` is issued. */
  onSendCommand: (text: string) => void;
  /** Open this coach's Discover edit sheet, `discover/<coachId>`. */
  onEditCoach: (coachId: string) => void;
}

/**
 * Who this thread is talking to.
 *
 * The coach is not managed from chat: t('chat.removeFromChat') sends
 * `/coach remove` down the same command pipeline a typed command takes, and
 * editing leaves for the coach's Discover detail, which owns the edit sheet.
 * A system coach belongs to the catalogue, so it offers no edit.
 */
export default function CoachInfoPanel({
  coach,
  onSendCommand,
  onEditCoach,
}: CoachInfoPanelProps) {
  const { t } = useTranslation();
  return (
    <div className="space-y-5" data-testid="coach-info-panel">
      <section>
        <h4 className="text-xs font-semibold uppercase tracking-wide text-outline mb-1">{t('chat.coachPanelTitle')}</h4>
        <p className="text-base font-semibold text-on-surface">{coach.title}</p>
        {coach.handle && (
          <p className="text-sm text-primary font-mono mt-0.5" data-testid="coach-info-handle">
            @{coach.handle}
          </p>
        )}
        <p className="text-xs text-outline mt-1">{coach.category}</p>
      </section>

      {coach.description && (
        <section>
          <h4 className="text-xs font-semibold uppercase tracking-wide text-outline mb-1">
            {t('chat.whatItDoesSection')}
          </h4>
          <p className="text-sm text-on-surface-variant">{coach.description}</p>
        </section>
      )}

      {coach.handle && (
        <section className="rounded-lg bg-surface-container-low p-3">
          <p className="flex items-start gap-2 text-xs text-on-surface-variant">
            <AtSign className="w-3.5 h-3.5 mt-0.5 flex-shrink-0 text-primary" aria-hidden="true" />
            <span>
              {t('chat.mentionLabel')} <span className="font-mono text-on-surface">@{coach.handle}</span> {t('chat.mentionHint')}
            </span>
          </p>
        </section>
      )}

      <section className="space-y-2">
        <Button
          variant="secondary"
          className="w-full"
          onClick={() => onSendCommand(COMMAND_DRAFTS.coachRemove)}
          data-testid="coach-info-remove"
        >
          <span className="flex items-center justify-center gap-2">
            <UserMinus className="w-4 h-4" aria-hidden="true" />
            {t('chat.removeFromChat')}
          </span>
        </Button>
        {!coach.is_system && (
          <Button
            variant="secondary"
            className="w-full"
            onClick={() => onEditCoach(coach.id)}
            data-testid="coach-info-edit"
          >
            <span className="flex items-center justify-center gap-2">
              <Pencil className="w-4 h-4" aria-hidden="true" />
              {t('chat.editCoach')}
            </span>
          </Button>
        )}
      </section>
    </div>
  );
}
