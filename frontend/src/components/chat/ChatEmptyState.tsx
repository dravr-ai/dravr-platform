// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What the chat pane shows with no thread open — one line, the "+" menu and the "/" commands button
// ABOUTME: The discoverable path to the command palette, so "/" is never the only way to find it

import type { ReactNode } from 'react';
import { MessageCircle, Slash } from 'lucide-react';
import { SLASH_HINT } from '@pierre/shared-constants';
import { Button } from '../ui';
import { useTranslation } from '@pierre/i18n';

interface ChatEmptyStateProps {
  /**
   * The chat "+" menu, rendered by the host so it stays wired to the one
   * conversation-creating mutation rather than a second copy of it.
   */
  compose: ReactNode;
  /** Start a conversation whose composer already holds `/`, palette open. */
  onOpenCommands: () => void;
  /** Disables the commands button while a conversation is being created. */
  disabled?: boolean;
}

/**
 * The empty chat pane.
 *
 * One line naming what to do, the `+` that starts a conversation, and a
 * `Commands` button that opens a thread with `/` already typed — the visible
 * affordance the slash palette otherwise lacks.
 */
export default function ChatEmptyState({
  compose,
  onOpenCommands,
  disabled = false,
}: ChatEmptyStateProps) {
  const { t } = useTranslation();
  return (
    <div
      className="flex-1 flex flex-col items-center justify-center gap-4 px-6 py-10 text-center"
      data-testid="chat-empty-state"
    >
      <div className="w-12 h-12 rounded-full bg-surface-container-low flex items-center justify-center">
        <MessageCircle className="w-6 h-6 text-primary" aria-hidden="true" />
      </div>
      <p className="text-base text-on-surface">{t('chat.emptyStatePrompt')}</p>
      <div className="flex items-center gap-2">
        {compose}
        <Button
          variant="secondary"
          onClick={onOpenCommands}
          disabled={disabled}
          data-testid="chat-empty-commands"
        >
          <span className="flex items-center gap-1.5">
            <Slash className="w-4 h-4" aria-hidden="true" />
            {t('chat.commandsButton')}
          </span>
        </Button>
      </div>
      <p className="text-xs text-outline max-w-sm">{SLASH_HINT}</p>
    </div>
  );
}
