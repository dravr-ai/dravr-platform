// ABOUTME: Onboarding step — pick which messaging app to chat with the coach on (Telegram/WhatsApp/Slack/…)
// ABOUTME: Shown only when the tenant has >1 channel configured; a single channel is auto-selected upstream

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import type { AvailableChannel } from '@pierre/api-client';
import { Button } from './ui';
import OnboardingShell from './OnboardingShell';

/**
 * Messaging-channel picker. The user chooses which chat app to connect so they
 * can talk to their coach there. Only reached when more than one channel is
 * configured for the tenant (a single channel is auto-selected by the flow, so
 * we never ask someone to pick from a list of one).
 */
export default function OnboardingMessagingChannel({
  userDisplayName,
  channels,
  onSelect,
  onSkip,
}: {
  userDisplayName?: string | null;
  channels: AvailableChannel[];
  onSelect: (channel: string) => void;
  onSkip: () => void;
}) {
  const [choosing, setChoosing] = useState<string | null>(null);

  const handleSelect = (channel: string) => {
    if (choosing) return;
    setChoosing(channel);
    onSelect(channel);
  };

  return (
    <OnboardingShell
      heading={userDisplayName ? `Almost there, ${userDisplayName}` : 'Almost there'}
    >
      <p className="mt-3 text-sm text-on-surface-variant font-label text-center">
        Where do you want to chat with your coach? Pick the app you already use — you can add
        more later in Settings.
      </p>
      <div className="mt-8 grid gap-4 sm:grid-cols-2">
        {channels.map((channel) => (
          <ChannelCard
            key={channel.channel}
            channel={channel}
            busy={choosing === channel.channel}
            disabled={choosing !== null}
            onSelect={() => handleSelect(channel.channel)}
          />
        ))}
      </div>
      <div className="mt-8">
        <Button variant="secondary" onClick={onSkip} className="w-full" disabled={choosing !== null}>
          Skip for now
        </Button>
      </div>
    </OnboardingShell>
  );
}

/** A single selectable messaging channel, rendered as a full-height button. */
function ChannelCard({
  channel,
  busy,
  disabled,
  onSelect,
}: {
  channel: AvailableChannel;
  busy: boolean;
  disabled: boolean;
  onSelect: () => void;
}) {
  const hint =
    channel.method === 'deep_link'
      ? 'Scan a QR or tap to open the app'
      : 'Connect with one tap';
  return (
    <button
      type="button"
      onClick={onSelect}
      disabled={disabled}
      className="flex flex-col items-start gap-2 rounded-xl border border-outline-variant bg-surface-container-low px-5 py-5 text-left transition-colors hover:border-primary hover:bg-surface-container disabled:cursor-not-allowed disabled:opacity-60"
    >
      <span className="flex items-center gap-2">
        <span className="font-display font-semibold text-base text-on-surface">
          {channel.display_name}
        </span>
        {channel.recommended ? (
          <span className="rounded-full bg-primary/15 px-2 py-0.5 text-[11px] font-medium text-primary">
            Recommended
          </span>
        ) : null}
      </span>
      <span className="text-sm text-on-surface-variant font-label">{hint}</span>
      {busy ? (
        <span className="mt-1 flex items-center gap-2 text-xs text-on-surface-variant">
          <span className="pierre-spinner w-4 h-4 border-on-surface border-t-transparent" />
          Setting things up…
        </span>
      ) : null}
    </button>
  );
}
