// ABOUTME: The chat-app link inside the chat-app sheet — QR + deep link (Telegram/WhatsApp) or an OAuth redirect, then the wait
// ABOUTME: Polls GET /api/messaging/links and reports the link the moment it lands; « Utiliser une autre app » goes back to the choice

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect } from 'react';
import { useQuery } from '@tanstack/react-query';
import { messagingLinkApi } from '../../services/api';
import { CHANNEL_LINK_POLL_INTERVAL_MS } from '@pierre/shared-constants';
import { Button } from '../ui';
import ChannelLinkPanel from '../ChannelLinkPanel';
import { useTranslation } from '@pierre/i18n';

/**
 * Connect the chosen messaging channel.
 *
 * Deep-link channels (Telegram/WhatsApp) show a QR — the primary path on
 * desktop, where the person runs the chat app on their phone — plus a tap
 * button, the only path on a phone. OAuth channels (Slack/Discord) show a
 * single connect button that redirects. Either way we poll the person's linked
 * channels and report the instant the link lands, so nobody has to come back
 * and click "done". « Utiliser une autre app » is offered when the workspace
 * has another app to pick.
 */
export default function ChatAppConnect({
  channel,
  displayName,
  showQr,
  onLinked,
  onUseAnother,
}: {
  channel: string;
  displayName: string;
  /** Whether the deep link is offered as a QR — never on the phone that would have to scan it. */
  showQr: boolean;
  onLinked: () => void;
  /** Back to the app choice; absent when the workspace offers only this app. */
  onUseAnother?: () => void;
}) {
  const { t } = useTranslation();
  // One link-init per channel: re-initialising would mint a fresh code and
  // invalidate the QR the person may be mid-scan of, so never refetch it.
  const { data: link, isLoading, isError, refetch, isFetching } = useQuery({
    queryKey: ['messaging-link-init', channel],
    queryFn: () => messagingLinkApi.initLink(channel),
    staleTime: Infinity,
    retry: 1,
  });

  // Poll the person's linked channels — the desktop can only learn the phone
  // finished by asking. The wait is what is being polled for, so the interval
  // ends with it: once this channel appears the answer cannot change again,
  // and a poll that kept running would bill an instance for a sheet the
  // person has already left.
  const { data: links } = useQuery({
    queryKey: ['messaging-links'],
    queryFn: () => messagingLinkApi.listLinks(),
    refetchInterval: query =>
      query.state.data?.some((l) => l.channel === channel) ? false : CHANNEL_LINK_POLL_INTERVAL_MS,
  });

  useEffect(() => {
    if (links?.some((l) => l.channel === channel)) {
      onLinked();
    }
  }, [links, channel, onLinked]);

  const useAnother = onUseAnother ? (
    <button
      type="button"
      onClick={onUseAnother}
      data-testid="chat-app-use-another"
      className="mx-auto block rounded text-sm font-medium text-primary hover:underline focus-ring"
    >
      {t('chatApp.useAnother')}
    </button>
  ) : null;

  if (isLoading) {
    return (
      <div className="flex flex-col items-center gap-4 py-8" role="status">
        <div className="pierre-spinner w-10 h-10 border-on-surface border-t-transparent" />
        <p className="text-sm text-on-surface">{t('app.preparingLink', { channel: displayName })}</p>
      </div>
    );
  }

  if (isError || !link) {
    return (
      <div className="flex flex-col items-center gap-4 py-8">
        <p className="text-sm text-on-surface" role="alert">
          {t('app.couldNotStartConnection', { channel: displayName })}
        </p>
        <Button variant="secondary" onClick={() => void refetch()} disabled={isFetching}>
          {t('onboarding.retry')}
        </Button>
        {useAnother}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-6">
      <ChannelLinkPanel link={link} displayName={displayName} showQr={showQr} />
      {useAnother}
    </div>
  );
}
