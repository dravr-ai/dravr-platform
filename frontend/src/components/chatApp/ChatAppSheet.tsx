// ABOUTME: The chat-app sheet — pick Telegram, WhatsApp or another app the workspace offers, then link it, over the page
// ABOUTME: Opened from Home's Getting started row and a coach's group card; the link landing closes it and says so (carnet#1525)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useCallback, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { CHAT_APP_OFFER_STEP_ID, QUERY_KEYS } from '@pierre/shared-constants';
import { userApi } from '../../services/api';
import { useAvailableChannels } from '../../hooks/useAvailableChannels';
import { Sheet } from '../ui/Sheet';
import { EmptyState, useSuccessToast } from '../ui';
import ChatAppPicker from './ChatAppPicker';
import ChatAppConnect from './ChatAppConnect';

interface ChatAppSheetProps {
  /** A bottom sheet on a phone, a side panel on a wider screen. */
  side: 'bottom' | 'right';
  /**
   * The app the offer named — the coach's card says « Configurer Telegram »,
   * so the sheet opens on Telegram's link. Without one the sheet opens on the
   * choice, or on the only app the workspace offers.
   */
  initialChannel?: string;
  onClose: () => void;
}

/**
 * Link a chat app, so the person can write to their agent from the messaging
 * app they already use — the two screens that closed the onboarding wizard,
 * moved over the page they lead to.
 *
 * The choice comes first when the workspace offers more than one app; one
 * app goes straight to its link, and « Utiliser une autre app » goes back to
 * the choice. On a phone the link is the "Open" button alone: the QR exists to
 * reach a phone from a desktop, and nobody can scan the screen they hold.
 * When the link lands the sheet closes with a toast, the offer's record is
 * written `complete` (so the group card does not come back) and Getting
 * started re-reads its rows. Closing the sheet answers nothing.
 */
export function ChatAppSheet({ side, initialChannel, onClose }: ChatAppSheetProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const toast = useSuccessToast();
  // Opening the sheet re-reads what the workspace offers, as Settings does.
  const { channels, query } = useAvailableChannels({ staleTime: 0 });
  const [picked, setPicked] = useState<string | null>(initialChannel ?? null);
  const linkedOnce = useRef(false);
  const showQr = side !== 'bottom';

  // An app the workspace stopped offering since the card was drawn falls back
  // to the choice rather than to a link that cannot start.
  const only = channels.length === 1 ? channels[0].channel : null;
  const chosen = channels.some((c) => c.channel === picked) ? picked : only;
  const chosenChannel = channels.find((c) => c.channel === chosen);
  const displayName = chosenChannel?.display_name ?? chosen ?? '';

  const onLinked = useCallback(() => {
    // The poll can report the same landed link on more than one render.
    if (linkedOnce.current) return;
    linkedOnce.current = true;
    void userApi.setOnboardingStep(CHAT_APP_OFFER_STEP_ID, 'complete').catch(() => {});
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.onboardingStatus() });
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.home.gettingStarted() });
    toast(t('chatApp.linkedToast', { app: displayName }));
    onClose();
  }, [displayName, onClose, queryClient, t, toast]);

  const title = chosen ? t('app.connectChannelTitle', { channel: displayName }) : t('chatApp.pickTitle');

  let body;
  if (query.isLoading) {
    body = (
      <div className="flex justify-center py-8" role="status">
        <div className="pierre-spinner w-6 h-6" />
      </div>
    );
  } else if (channels.length === 0) {
    // The offers are drawn only for a workspace with an app; this covers one
    // that was switched off between the offer and the tap.
    body = <EmptyState data-testid="chat-app-none">{t('app.noChatAppsConfigured')}</EmptyState>;
  } else if (chosen === null) {
    body = <ChatAppPicker channels={channels} onSelect={setPicked} showQr={showQr} />;
  } else {
    body = (
      <ChatAppConnect
        key={chosen}
        channel={chosen}
        displayName={displayName}
        showQr={showQr}
        onLinked={onLinked}
        onUseAnother={channels.length > 1 ? () => setPicked(null) : undefined}
      />
    );
  }

  return (
    <Sheet side={side} title={title} onClose={onClose} data-testid="chat-app-sheet">
      <div className="px-4 py-4">{body}</div>
    </Sheet>
  );
}

export default ChatAppSheet;
