// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The pill between two days of a thread — today, yesterday, or the date spelled out
// ABOUTME: A separator to assistive tech, a quiet chip on the canvas to everyone else

import type { DayLabel } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';

/** The pill for `day`: today and yesterday in the reader's words, any earlier day as its date. */
export default function DaySeparator({ day }: { day: DayLabel }) {
  const { t } = useTranslation();
  const label =
    day.kind === 'today' ? t('chat.dayToday') : day.kind === 'yesterday' ? t('chat.dayYesterday') : day.label;
  return (
    <div role="separator" aria-label={label} data-testid="day-separator" className="my-3 flex justify-center">
      <span className="text-xs font-medium text-outline">
        {label}
      </span>
    </div>
  );
}
