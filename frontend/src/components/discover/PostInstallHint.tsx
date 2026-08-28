// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The card shown after a coach is installed from Discover — teaches /coach add @handle and @handle
// ABOUTME: Dismissible; t('app.openChat') hands the /coach add draft to the caller and starts a conversation

import { Button, Card } from '../ui';
import { coachAddDraft, coachMention } from './coachDraft';
import { useTranslation } from '@pierre/i18n';

export interface PostInstallHintProps {
  coachTitle: string;
  /** The catalogue handle the copy inherited from its listing. */
  handle: string | undefined;
  /**
   * Receives the `/coach add @handle` command so the caller can start a
   * conversation and seed its composer with it.
   */
  onOpenChat: (draft: string) => void;
  onDismiss: () => void;
}

export default function PostInstallHint({ coachTitle, handle, onOpenChat, onDismiss }: PostInstallHintProps) {
  const { t } = useTranslation();
  const draft = coachAddDraft(handle);
  const mention = coachMention(handle);
  return (
    <section data-testid="post-install-hint" aria-live="polite">
      <Card variant="dark" className="space-y-3">
        <h3 className="text-base font-semibold text-on-surface">
          &ldquo;{coachTitle}&rdquo; is in your coaches
        </h3>
        <p className="text-sm text-on-surface-variant">
          {t('discover.postInstallUseHint')} <code className="font-mono text-primary">{draft}</code> — or mention{' '}
          <code className="font-mono text-primary">{mention}</code> {t('frag.forOneTurn')}
        </p>
        <div className="flex flex-wrap gap-2">
          <Button size="sm" onClick={() => onOpenChat(draft)} data-testid="post-install-open-chat">
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
