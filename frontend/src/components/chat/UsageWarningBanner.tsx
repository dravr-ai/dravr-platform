// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Dismissible usage warning banner for the chat interface
// ABOUTME: Shows yellow/orange/red warnings based on quota usage percentage

import { useState } from 'react';
import { AlertTriangle, X, Ban } from 'lucide-react';
import type { WarningLevel } from '../../hooks/useUsageStatus';
import { useTranslation } from '@pierre/i18n';

interface UsageWarningBannerProps {
  /** The warning severity level */
  level: WarningLevel;
  /** The message to display */
  message: string;
}

// Three escalation steps on a four-token palette, so the ladder is built from
// emphasis rather than a fourth hue: warning reads as a quiet tint, burst keeps
// the same hue at roughly double the weight, blocked switches to error. Giving
// warning and burst identical styling would make the escalation invisible.
const BANNER_STYLES: Record<Exclude<WarningLevel, 'none'>, { bg: string; border: string; text: string; icon: string }> = {
  warning: {
    bg: 'bg-warning/10',
    border: 'border-warning/30',
    text: 'text-warning',
    icon: 'text-warning',
  },
  burst: {
    bg: 'bg-warning/25',
    border: 'border-warning/60',
    text: 'text-warning',
    icon: 'text-warning',
  },
  blocked: {
    bg: 'bg-error/10',
    border: 'border-error/30',
    text: 'text-error',
    icon: 'text-error',
  },
};

export default function UsageWarningBanner({ level, message }: UsageWarningBannerProps) {
  const { t } = useTranslation();
  const [dismissed, setDismissed] = useState(false);

  if (level === 'none' || dismissed || !message) {
    return null;
  }

  const styles = BANNER_STYLES[level];
  const Icon = level === 'blocked' ? Ban : AlertTriangle;

  return (
    <div
      role="alert"
      data-testid="usage-warning-banner"
      className={`flex items-center gap-3 px-4 py-2.5 ${styles.bg} border-b ${styles.border} ${styles.text} text-sm`}
    >
      <Icon className={`w-4 h-4 flex-shrink-0 ${styles.icon}`} />
      <span className="flex-1">{message}</span>
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
