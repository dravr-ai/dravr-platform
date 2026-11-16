// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Dismissible usage warning banner for the chat interface
// ABOUTME: Draws the warning, burst and blocked levels through the alert tone table

import { useState } from 'react';
import { AlertTriangle, X, Ban } from 'lucide-react';
import type { WarningLevel } from '../../hooks/useUsageStatus';
import { useTranslation } from '@pierre/i18n';
import type { TranslatableText } from '@pierre/chat-utils';
import { ALERT_TONE } from '../ui';

interface UsageWarningBannerProps {
  /** The warning severity level */
  level: WarningLevel;
  /** The sentence, as a catalogue key plus params, or `null` when silent. */
  text: TranslatableText | null;
}

// Three escalation steps on the alert table, so every level's ink is the one
// measured on its tint (DESIGN.md "Alerts"): warning is the warning alert,
// burst keeps that tone and doubles its edge, blocked switches to error. A
// deeper tint (/25) would drop the warning ink under AA on the upper tiers;
// the edge is not text, so it carries the weight without costing contrast. Giving warning and burst identical styling
// would make the escalation invisible.
const BANNER_STYLES: Record<Exclude<WarningLevel, 'none'>, string> = {
  warning: ALERT_TONE.warning.tone,
  burst: `${ALERT_TONE.warning.tone} border-warning/60`,
  blocked: ALERT_TONE.error.tone,
};

export default function UsageWarningBanner({ level, text }: UsageWarningBannerProps) {
  const { t } = useTranslation();
  const [dismissed, setDismissed] = useState(false);

  if (level === 'none' || dismissed || text === null) {
    return null;
  }

  const styles = BANNER_STYLES[level];
  const Icon = level === 'blocked' ? Ban : AlertTriangle;

  return (
    <div
      role="alert"
      data-testid="usage-warning-banner"
      className={`flex items-center gap-3 px-4 py-2.5 border-b text-sm ${styles}`}
    >
      <Icon className="w-4 h-4 flex-shrink-0" />
      <span className="flex-1">{t(text.key, text.params)}</span>
      {level !== 'blocked' && (
        <button
          onClick={() => setDismissed(true)}
          className="flex-shrink-0 p-0.5 rounded hover:bg-surface-container transition-colors"
          aria-label={t('chat.dismissWarning')}
        >
          <X className="w-3.5 h-3.5" />
        </button>
      )}
    </div>
  );
}
