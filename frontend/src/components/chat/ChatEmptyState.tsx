// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What the thread pane shows with no thread open — the brand mark, one invitation, the "+" and three quick ways in
// ABOUTME: The discoverable path to the command palette, so "/" is never the only way to find it

import type { ReactNode } from 'react';
import { clsx } from 'clsx';
import { Compass, Link2, Slash } from 'lucide-react';
import { SLASH_HINT_KEY } from '@pierre/shared-constants';
import { CONNECTIONS_ROUTE } from '../../constants/surfaceLayout';
import { DravrLogo } from '../DravrLogo';
import { useTranslation } from '@pierre/i18n';

interface ChatEmptyStateProps {
  /**
   * The chat "+" menu, rendered by the host so it stays wired to the one
   * conversation-creating mutation rather than a second copy of it.
   */
  compose: ReactNode;
  /** Start a conversation whose composer already holds `/`, palette open. */
  onOpenCommands: () => void;
  /** Disables the quick actions while a conversation is being created. */
  disabled?: boolean;
  /** Jump to another tab — the coach catalogue, the data providers. */
  onNavigate?: (route: string) => void;
  /** What the coach can see right now: the connected providers, or that there are none. */
  providerStatus?: string | null;
}

/** One of the round quick actions under the invitation. */
function QuickAction({
  icon,
  label,
  onClick,
  disabled,
  testId,
}: {
  icon: ReactNode;
  label: string;
  onClick: () => void;
  disabled?: boolean;
  testId?: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      data-testid={testId}
      className="flex w-24 flex-col items-center gap-2 rounded-xl px-2 py-2 text-on-surface transition-colors hover:bg-surface-container-high disabled:cursor-not-allowed disabled:opacity-50 focus-ring"
    >
      <span className="flex h-12 w-12 items-center justify-center rounded-full bg-surface-container-high text-primary">
        {icon}
      </span>
      <span className="text-center text-xs leading-tight">{label}</span>
    </button>
  );
}

/**
 * The empty thread pane.
 *
 * A centred card on the canvas: the mark, one line naming what to do, the
 * `+` that starts a conversation as the single call to action, and three
 * quick ways in below it — the command palette, the coach catalogue and the
 * data providers. The footer names what the coach can see, so an athlete with
 * nothing connected learns it here rather than after the first question.
 */
export default function ChatEmptyState({
  compose,
  onOpenCommands,
  disabled = false,
  onNavigate,
  providerStatus,
}: ChatEmptyStateProps) {
  const { t } = useTranslation();
  return (
    <div
      className="flex flex-1 flex-col items-center justify-center overflow-y-auto px-6 py-10"
      data-testid="chat-empty-state"
    >
      <div className="flex w-full max-w-md flex-col items-center rounded-xl border ghost-border bg-surface-container-lowest px-8 py-10 text-center shadow-card">
        <DravrLogo size={72} />
        <h2 className="mt-6 font-display text-xl font-semibold text-on-surface">
          {t('chat.emptyStatePrompt')}
        </h2>
        <p className="mt-2 text-sm text-on-surface-variant">{t('chat.emptyStateBody')}</p>
        <div className="mt-6 flex items-center justify-center">{compose}</div>
      </div>
      <div className="mt-8 flex flex-wrap items-start justify-center gap-2">
        <QuickAction
          icon={<Slash className="h-5 w-5" aria-hidden="true" />}
          label={t('chat.commandsButton')}
          onClick={onOpenCommands}
          disabled={disabled}
          testId="chat-empty-commands"
        />
        {onNavigate && (
          <>
            <QuickAction
              icon={<Compass className="h-5 w-5" aria-hidden="true" />}
              label={t('chat.quickDiscover')}
              onClick={() => onNavigate('discover')}
              testId="chat-empty-discover"
            />
            <QuickAction
              icon={<Link2 className="h-5 w-5" aria-hidden="true" />}
              label={t('chat.quickConnectProvider')}
              onClick={() => onNavigate(CONNECTIONS_ROUTE)}
              testId="chat-empty-connect"
            />
          </>
        )}
      </div>
      {providerStatus ? (
        <p className="mt-6 text-xs text-on-surface-variant" data-testid="chat-empty-provider-status">
          {providerStatus}
        </p>
      ) : null}
      <p className={clsx('max-w-sm text-xs text-outline', providerStatus ? 'mt-1' : 'mt-6')}>
        {t(SLASH_HINT_KEY)}
      </p>
    </div>
  );
}
