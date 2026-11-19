// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info's one-time notice to a member sharing with a human coach: the AI tools the coach connects read them by name
// ABOUTME: Its two answers are the coach-sharing switch's own writes — keep or stop — and either, made in the app, records the answer

import { useId } from 'react';
import { COACH_ASSISTANT_NOTICE_KEYS } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { Button, ALERT_TONE } from '../ui';

interface CoachAssistantNoticeProps {
  /** The group's coach as the server names them. */
  coach: string;
  /** A coach-sharing write is on its way, so neither answer can be given again. */
  disabled: boolean;
  /** The member's answer: keep sharing (`true`) or stop (`false`). */
  onAnswer: (keep: boolean) => void;
}

/**
 * Told once to a member sharing with a human coach who has not answered it
 * yet (`assistant_notice_due`), as the wellness widening is told: AI tools the
 * coach connects to Dravr read the member by name, the company running each
 * receives what it reads and may process it outside Québec. Keeping writes
 * the sharing again; stopping ends the coach's access and their tools' alike.
 * Either write, made here in Dravr's app, records the answer and the server's
 * reply clears the notice; a change made over an API key does not.
 */
export default function CoachAssistantNotice({ coach, disabled, onAnswer }: CoachAssistantNoticeProps) {
  const { t } = useTranslation();
  const titleId = useId();
  return (
    <div
      role="note"
      aria-labelledby={titleId}
      className={`${ALERT_TONE.info.tone} space-y-2 rounded-lg border p-3`}
      data-testid="coach-assistant-notice"
    >
      <p id={titleId} className="text-sm font-medium">
        {t(COACH_ASSISTANT_NOTICE_KEYS.title)}
      </p>
      <p className="text-sm">{t(COACH_ASSISTANT_NOTICE_KEYS.body, { coach })}</p>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          variant="tertiary"
          size="sm"
          disabled={disabled}
          onClick={() => onAnswer(true)}
          data-testid="coach-assistant-notice-keep"
        >
          {t(COACH_ASSISTANT_NOTICE_KEYS.keep)}
        </Button>
        <Button
          variant="tertiary"
          size="sm"
          className="text-error"
          disabled={disabled}
          onClick={() => onAnswer(false)}
          data-testid="coach-assistant-notice-stop"
        >
          {t(COACH_ASSISTANT_NOTICE_KEYS.stop)}
        </Button>
      </div>
    </div>
  );
}
