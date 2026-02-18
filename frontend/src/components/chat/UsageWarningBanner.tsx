// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Dismissible usage warning banner for the chat interface
// ABOUTME: Shows yellow/orange/red warnings based on quota usage percentage

import { useState } from 'react';
import { AlertTriangle, X, Ban } from 'lucide-react';
import type { WarningLevel } from '../../hooks/useUsageStatus';

interface UsageWarningBannerProps {
  /** The warning severity level */
  level: WarningLevel;
  /** The message to display */
  message: string;
}

const BANNER_STYLES: Record<Exclude<WarningLevel, 'none'>, { bg: string; border: string; text: string; icon: string }> = {
  warning: {
    bg: 'bg-yellow-500/10',
    border: 'border-yellow-500/30',
    text: 'text-yellow-300',
    icon: 'text-yellow-400',
  },
  burst: {
    bg: 'bg-orange-500/10',
    border: 'border-orange-500/30',
    text: 'text-orange-300',
    icon: 'text-orange-400',
  },
  blocked: {
    bg: 'bg-pierre-red-500/10',
    border: 'border-pierre-red-500/30',
    text: 'text-pierre-red-400',
    icon: 'text-pierre-red-400',
  },
};

export default function UsageWarningBanner({ level, message }: UsageWarningBannerProps) {
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
          className="flex-shrink-0 p-0.5 rounded hover:bg-white/10 transition-colors"
          aria-label="Dismiss warning"
        >
          <X className="w-3.5 h-3.5" />
        </button>
      )}
    </div>
  );
}
