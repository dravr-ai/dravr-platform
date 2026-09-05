// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What the thread pane shows with no thread open — the mark, one invitation, and three ink links: the "+", commands, an agent, your data
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
  /** Jump to another tab — the agent catalogue, the data providers. */
  onNavigate?: (route: string) => void;
  /** What the agent can see right now: the connected providers, or that there are none. */
  providerStatus?: string | null;
}

/** One of the ink links under the invitation. */
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
      className="inline-flex items-center gap-1.5 rounded text-sm font-medium text-primary transition-colors hover:text-primary-hover disabled:cursor-not-allowed disabled:opacity-50 focus-ring touch-target"
    >
      <span className="text-primary [&_svg]:h-4 [&_svg]:w-4">{icon}</span>
      <span>{label}</span>
    </button>
  );
}

/**
 * The empty thread pane.
 *
 * Boreal v2 put a white card here — the mark, a headline, a filled "+" —
 * with three grey circles and two helper lines under it: more chrome than
 * any thread. v2.1 leaves the canvas bare. A 420px column, left-aligned and
 * vertically centred: the mark, one 18px line, one line of body, then the
 * ways in as ink links in a row, and the provider line in the caption size
 * with its action inline. The measured floor for this screen is a headline
 * and a composer; nothing here is boxed or filled.
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
      <div className="flex w-full max-w-[520px] flex-col items-start">
        <DravrLogo size={44} />
        <h2 className="mt-5 font-display text-xl font-semibold text-on-surface">{t('chat.emptyStatePrompt')}</h2>
        <p className="mt-1 text-sm text-on-surface-variant">{t('chat.emptyStateBody')}</p>
        <div className="mt-5 flex flex-wrap items-center gap-x-5 gap-y-2">
          <div className="flex items-center">{compose}</div>
          <QuickAction
            icon={<Slash aria-hidden="true" />}
            label={t('chat.commandsButton')}
            onClick={onOpenCommands}
            disabled={disabled}
            testId="chat-empty-commands"
          />
          {onNavigate && (
            <>
              <QuickAction
                icon={<Compass aria-hidden="true" />}
                label={t('chat.quickDiscover')}
                onClick={() => onNavigate('discover')}
                testId="chat-empty-discover"
              />
              <QuickAction
                icon={<Link2 aria-hidden="true" />}
                label={t('chat.quickConnectProvider')}
                onClick={() => onNavigate(CONNECTIONS_ROUTE)}
                testId="chat-empty-connect"
              />
            </>
          )}
        </div>
        {/* The two caption lines: what the agent can see, and the one grammar
            lesson — `/` and `@` — that has no other home before the first
            message. Both in the caption size, neither boxed. */}
        {providerStatus ? (
          <p className="mt-7 text-xs text-outline" data-testid="chat-empty-provider-status">
            {providerStatus}
          </p>
        ) : null}
        <p className={clsx('text-xs text-outline', providerStatus ? 'mt-1' : 'mt-7')}>{t(SLASH_HINT_KEY)}</p>
      </div>
    </div>
  );
}
