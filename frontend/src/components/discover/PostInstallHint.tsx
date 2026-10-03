// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The card shown after an agent is installed from Discover — teaches /agent add @handle and @handle
// ABOUTME: Dismissible; t('discover.openChat') asks the caller for a thread bound to the agent, which welcomes

import { Button, Card } from '../ui';
import { coachAddDraft, coachMention } from './coachDraft';
import { useTranslation } from '@pierre/i18n';

export interface PostInstallHintProps {
  agentTitle: string;
  /** The catalogue handle the copy inherited from its listing. */
  handle: string | undefined;
  /** True while the caller creates the bound thread; Open chat waits on it. */
  isOpeningChat?: boolean;
  /**
   * Starts a conversation bound to the installed copy, so the agent's welcome
   * is in it. The `/agent add @handle` the hint teaches is for any other chat.
   */
  onOpenChat: () => void;
  onDismiss: () => void;
}

export default function PostInstallHint({
  agentTitle,
  handle,
  isOpeningChat = false,
  onOpenChat,
  onDismiss,
}: PostInstallHintProps) {
  const { t } = useTranslation();
  const draft = coachAddDraft(handle);
  const mention = coachMention(handle);
  return (
    <section data-testid="post-install-hint" aria-live="polite">
      <Card variant="dark" className="space-y-3">
        <h3 className="text-base font-semibold text-on-surface">
          &ldquo;{agentTitle}&rdquo; is in your agents
        </h3>
        <p className="text-sm text-on-surface-variant">
          {t('discover.postInstallUseHint')} <code className="font-mono text-primary">{draft}</code> — or mention{' '}
          <code className="font-mono text-primary">{mention}</code> {t('frag.forOneTurn')}
        </p>
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            onClick={() => onOpenChat()}
            loading={isOpeningChat}
            data-testid="post-install-open-chat"
          >
            {t('discover.openChat')}
          </Button>
          <Button size="sm" variant="secondary" onClick={onDismiss} data-testid="post-install-dismiss">
            {t('chat.dismiss')}
          </Button>
        </div>
      </Card>
    </section>
  );
}
