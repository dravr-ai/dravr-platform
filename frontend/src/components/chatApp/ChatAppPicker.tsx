// ABOUTME: The chat-app choice inside the chat-app sheet — which messaging app to talk to the agent on (Telegram, WhatsApp, …)
// ABOUTME: Once the wizard's picker step; now the sheet's first view, shown only when the workspace offers more than one app

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import type { AvailableChannel } from '@pierre/api-client';
import { useTranslation } from '@pierre/i18n';

/**
 * Messaging-channel picker. The person chooses which chat app to connect so
 * they can talk to their agent there. The sheet opens on it only when more
 * than one app is offered (a single app goes straight to its link), and comes
 * back to it from « Utiliser une autre app ». Closing the sheet is the skip.
 */
export default function ChatAppPicker({
  channels,
  onSelect,
  showQr,
}: {
  channels: AvailableChannel[];
  onSelect: (channel: string) => void;
  /** Whether a deep link is offered as a QR here — never on the phone that would have to scan it. */
  showQr: boolean;
}) {
  const { t } = useTranslation();
  const [choosing, setChoosing] = useState<string | null>(null);

  const handleSelect = (channel: string) => {
    if (choosing) return;
    setChoosing(channel);
    onSelect(channel);
  };

  return (
    <div data-testid="chat-app-picker">
      <p className="text-sm text-on-surface-variant">{t('onboarding.messagingChannelIntro')}</p>
      <ul className="mt-4">
        {channels.map((channel) => (
          <li key={channel.channel} className="border-t ghost-border-faint first:border-t-0">
            <ChannelRow
              channel={channel}
              busy={choosing === channel.channel}
              showQr={showQr}
              disabled={choosing !== null}
              onSelect={() => handleSelect(channel.channel)}
            />
          </li>
        ))}
      </ul>
    </div>
  );
}

/** A single selectable messaging channel: the app's name, how it links, and the row's one action. */
function ChannelRow({
  channel,
  busy,
  showQr,
  disabled,
  onSelect,
}: {
  channel: AvailableChannel;
  busy: boolean;
  showQr: boolean;
  disabled: boolean;
  onSelect: () => void;
}) {
  const { t } = useTranslation();
  const hint =
    channel.method === 'deep_link' && showQr
      ? t('onboarding.channelScanQr')
      : t('onboarding.channelOneTap');
  return (
    <button
      type="button"
      onClick={onSelect}
      disabled={disabled}
      data-testid={`chat-app-choose-${channel.channel}`}
      className="flex min-h-[52px] w-full items-center gap-3 px-1 py-3 text-left transition-colors hover:bg-surface-container-low focus-ring disabled:cursor-not-allowed disabled:opacity-60"
    >
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-2">
          <span className="text-sm font-medium text-on-surface">{channel.display_name}</span>
          {channel.recommended ? (
            <span className="rounded-full bg-primary/15 px-2 py-0.5 text-xs font-medium text-primary">
              {t('onboarding.channelRecommended')}
            </span>
          ) : null}
        </span>
        <span className="block text-xs text-on-surface-variant">{hint}</span>
      </span>
      {busy ? (
        <span className="flex shrink-0 items-center gap-2 text-xs text-on-surface-variant">
          <span className="pierre-spinner w-4 h-4 border-on-surface border-t-transparent" />
          {t('onboarding.channelSettingUp')}
        </span>
      ) : (
        <span className="shrink-0 text-sm font-medium text-primary">{t('app.connect')}</span>
      )}
    </button>
  );
}
