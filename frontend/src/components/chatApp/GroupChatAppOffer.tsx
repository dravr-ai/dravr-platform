// ABOUTME: The group agent's one-time chat-app card in a coach's group thread — « Écris-moi aussi depuis Telegram ? »
// ABOUTME: Configure opens the chat-app sheet; « Pas maintenant » stores the answer, so the card holds gone on every device (carnet#1525)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useTranslation } from '@pierre/i18n';
import MessageBubble from '../chat/MessageBubble';
import CoachAvatar from '../chat/CoachAvatar';
import type { GroupChatAppOfferState } from './useGroupChatAppOffer';

interface GroupChatAppOfferProps {
  offer: GroupChatAppOfferState;
  /** The group's agent as the thread names it: the card speaks in its voice. */
  agentLabel: string;
  /** The thread's last row is the agent's, so the card continues that run without its name again. */
  continuesRun?: boolean;
  /** « Configurer <app> »: open the chat-app sheet on that app. */
  onConfigure: (channel: string) => void;
}

/**
 * The card itself: an agent row at the end of the thread, a question, one
 * tonal action and a quiet « Pas maintenant » — an offer the coach may not
 * want, so nothing here is a filled button.
 */
export function GroupChatAppOffer({ offer, agentLabel, continuesRun = false, onConfigure }: GroupChatAppOfferProps) {
  const { t } = useTranslation();
  const [failed, setFailed] = useState(false);
  const [declining, setDeclining] = useState(false);
  const app = offer.app.display_name;

  const decline = () => {
    setFailed(false);
    setDeclining(true);
    offer
      .decline()
      .catch(() => setFailed(true))
      .finally(() => setDeclining(false));
  };

  return (
    <MessageBubble
      side="assistant"
      authorLabel={agentLabel}
      avatar={<CoachAvatar label={agentLabel} />}
      groupStart={!continuesRun}
    >
      <div data-testid="group-chat-app-offer" className="max-w-md">
        <p className="text-base font-medium leading-relaxed text-on-surface">
          {t('chatApp.offer.title', { app })}
        </p>
        <p className="mt-1 text-sm leading-relaxed text-on-surface-variant">{t('chatApp.offer.body', { app })}</p>
        <div className="mt-3 flex flex-wrap items-center gap-4">
          <button
            type="button"
            onClick={() => onConfigure(offer.app.channel)}
            disabled={declining}
            data-testid="group-chat-app-offer-configure"
            className="touch-target inline-flex items-center rounded-lg bg-primary/15 px-3 py-1.5 text-sm font-medium text-primary transition-colors hover:bg-primary/25 focus-ring disabled:opacity-50"
          >
            {t('chatApp.offer.configure', { app })}
          </button>
          <button
            type="button"
            onClick={decline}
            disabled={declining}
            data-testid="group-chat-app-offer-not-now"
            className="touch-target rounded text-sm font-medium text-on-surface-variant hover:text-on-surface hover:underline focus-ring disabled:opacity-50"
          >
            {t('chatApp.offer.notNow')}
          </button>
        </div>
        {failed ? (
          <p role="alert" className="mt-2 text-sm text-error">
            {t('chatApp.offer.notNowFailed')}
          </p>
        ) : null}
      </div>
    </MessageBubble>
  );
}
