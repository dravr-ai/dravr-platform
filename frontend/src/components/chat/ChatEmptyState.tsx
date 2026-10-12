// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What the thread pane shows with no thread open — the mark, one invitation, the "+" and three use-case links
// ABOUTME: The links are the starters ranked for the athlete; "/" and "@" are taught by the hint line under them

import type { ReactNode } from 'react';
import { clsx } from 'clsx';
import { SLASH_HINT_KEY } from '@pierre/shared-constants';
import type { UseCaseStarter } from '@pierre/shared-types';
import { DravrLogo } from '../DravrLogo';
import { useTranslation } from '@pierre/i18n';

interface ChatEmptyStateProps {
  /**
   * The chat "+" menu, rendered by the host so it stays wired to the one
   * conversation-creating mutation rather than a second copy of it.
   */
  compose: ReactNode;
  /**
   * The use-case starters ranked for the athlete — three at most — drawn as
   * the ink links. Empty while they load, or when none fits.
   */
  starters: UseCaseStarter[];
  /**
   * Start a thread on a starter: the host creates it and sends the starter's
   * postback as its first line, under the starter's label.
   */
  onStarter: (starter: UseCaseStarter) => void;
  /** Disables the starters while a conversation is being created. */
  disabled?: boolean;
  /** What the agent can see right now: the connected providers, or that there are none. */
  providerStatus?: string | null;
}

/**
 * The empty thread pane.
 *
 * Boreal v2.1 leaves the canvas bare: a 520px column, left-aligned and
 * vertically centred — the mark, one 18px line, one line of body, then the
 * "+" and the starters as ink links in a row, and the caption lines under
 * them. The links are use cases, not features (carnet#829): at most three
 * things this athlete can do now, ranked from what they have connected and
 * told us, worded as the athlete would ask. Nothing here is boxed or filled,
 * and no starter is a pill (D3).
 */
export default function ChatEmptyState({
  compose,
  starters,
  onStarter,
  disabled = false,
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
          {starters.map((starter) => (
            <button
              key={starter.id}
              type="button"
              onClick={() => onStarter(starter)}
              disabled={disabled}
              data-testid={`chat-empty-starter-${starter.id}`}
              className="inline-flex items-center rounded text-sm font-medium text-primary transition-colors hover:text-primary-hover disabled:cursor-not-allowed disabled:opacity-50 focus-ring touch-target"
            >
              {starter.label}
            </button>
          ))}
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
