// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info's one-time card asking a member sharing with a human coach whether the coach may be told of a break unusual for them
// ABOUTME: Its two answers are the routine-detection consent's own writes — turn on or not now — and the server stamps either, which clears the card

import { useId } from 'react';
import { ROUTINE_DETECTION_NOTICE_KEYS } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { Button, ALERT_TONE } from '../ui';

interface RoutineDetectionNoticeProps {
  /** The group's coach as the server names them. */
  coach: string;
  /** A coach-consents write is on its way, so neither answer can be given again. */
  disabled: boolean;
  /** The member's answer: turn routine detection on (`true`) or not now (`false`). */
  onAnswer: (turnOn: boolean) => void;
}

/**
 * Asked once of a member sharing with a human coach who has not answered it
 * yet (`routine_detection_notice_due`, carnet#1792): Dravr can learn their
 * pattern from their last 8 weeks of training and tell the coach when a break
 * is longer than any in it. Learning the pattern is profiling, so it stays
 * off until the member turns it on; the two answers carry the same weight.
 * "Turn on" grants `routine_detection`, "Not now" answers `false`; the server
 * stamps either, and its reply clears the card.
 */
export default function RoutineDetectionNotice({ coach, disabled, onAnswer }: RoutineDetectionNoticeProps) {
  const { t } = useTranslation();
  const titleId = useId();
  return (
    <div
      role="note"
      aria-labelledby={titleId}
      className={`${ALERT_TONE.info.tone} space-y-2 rounded-lg border p-3`}
      data-testid="routine-detection-notice"
    >
      <p id={titleId} className="text-sm font-medium">
        {t(ROUTINE_DETECTION_NOTICE_KEYS.title)}
      </p>
      <p className="text-sm">{t(ROUTINE_DETECTION_NOTICE_KEYS.body, { coach })}</p>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          variant="tertiary"
          size="sm"
          disabled={disabled}
          onClick={() => onAnswer(true)}
          data-testid="routine-detection-notice-turn-on"
        >
          {t(ROUTINE_DETECTION_NOTICE_KEYS.turnOn)}
        </Button>
        <Button
          variant="tertiary"
          size="sm"
          disabled={disabled}
          onClick={() => onAnswer(false)}
          data-testid="routine-detection-notice-not-now"
        >
          {t(ROUTINE_DETECTION_NOTICE_KEYS.notNow)}
        </Button>
      </div>
    </div>
  );
}
