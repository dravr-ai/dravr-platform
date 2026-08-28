// ABOUTME: Onboarding step — connect the chosen messaging app via QR + deep link (Telegram/WhatsApp) or OAuth redirect
// ABOUTME: Polls GET /api/messaging/links to auto-advance the moment the account link lands

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect } from 'react';
import { useQuery } from '@tanstack/react-query';
import { messagingLinkApi } from '../services/api';
import { CHANNEL_LINK_POLL_INTERVAL_MS } from '@pierre/shared-constants';
import { Button } from './ui';
import OnboardingShell from './OnboardingShell';
import { useTranslation } from '@pierre/i18n';

/**
 * Connect the chosen messaging channel.
 *
 * Deep-link channels (Telegram/WhatsApp) show a QR — the primary path on desktop,
 * where the user runs the chat app on their phone — plus a tap button (the primary
 * path on mobile). OAuth channels (Slack/Discord/Messenger) show a single connect
 * button that redirects. Either way we poll the user's linked channels and
 * auto-advance the instant the link lands, so the user never has to come back and
 * click "done".
 */
export default function OnboardingMessagingConfigure({
  userDisplayName,
  channel,
  displayName,
  onLinked,
  onSkip,
}: {
  userDisplayName?: string | null;
  channel: string;
  displayName: string;
  onLinked: () => void;
  onSkip: () => void;
}) {
  const { t } = useTranslation();
  // One link-init per channel: re-initialising would mint a fresh code and
  // invalidate the QR the user may be mid-scan of, so never refetch it.
  const { data: link, isLoading, isError, refetch, isFetching } = useQuery({
    queryKey: ['messaging-link-init', channel],
    queryFn: () => messagingLinkApi.initLink(channel),
    staleTime: Infinity,
    retry: 1,
  });

  // Poll the user's linked channels — the desktop can only learn the phone
  // finished by asking. The wait is what is being polled for, so the interval
  // ends with it: once this channel appears the answer cannot change again,
  // and a poll that kept running would bill an instance for a screen the
  // athlete has already left.
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

  if (isLoading) {
    return (
      <OnboardingShell heading={`Connect ${displayName}`}>
        <div className="flex flex-col items-center gap-4 py-8">
          <div className="pierre-spinner w-10 h-10 border-on-surface border-t-transparent" />
          <p className="text-sm text-on-surface font-label">{t('frag.preparingYour')} {displayName} link…</p>
        </div>
      </OnboardingShell>
    );
  }

  if (isError || !link) {
    return (
      <OnboardingShell heading={`Connect ${displayName}`}>
        <div className="flex flex-col items-center gap-4 py-8">
          <p className="text-sm text-on-surface font-label">
            {t('frag.couldntStartThe')} {displayName} connection just now.
          </p>
          <div className="flex gap-3">
            <Button variant="primary" onClick={() => void refetch()} disabled={isFetching}>
              {t('onboarding.retry')}
            </Button>
            <Button variant="secondary" onClick={onSkip}>
              {t('onboarding.skipForNow')}
            </Button>
          </div>
        </div>
      </OnboardingShell>
    );
  }

  const isDeepLink = link.method === 'deep_link';

  return (
    <OnboardingShell
      heading={userDisplayName ? t('app.obConnectGreeting', { channel: displayName, name: userDisplayName }) : `Connect ${displayName}`}
    >
      <div className="mt-6 flex flex-col items-center gap-5">
        {isDeepLink && link.qr_svg ? (
          <>
            {/* White plate so the black-on-white QR stays scannable in any theme. */}
            <div className="rounded-xl bg-white p-3">
              <img
                src={`data:image/svg+xml;utf8,${encodeURIComponent(link.qr_svg)}`}
                alt={t('frag.qrCodeFor', { app: displayName })}
                className="h-44 w-44"
              />
            </div>
            <p className="max-w-sm text-center text-sm text-on-surface-variant font-label">
              {t('onboarding.messagingScanPrefix', { app: displayName })}
              <span className="font-semibold"> {t('onboarding.messagingStartButton')}</span> {t('onboarding.messagingScanSuffix')}
            </p>
          </>
        ) : (
          <p className="max-w-sm text-center text-sm text-on-surface-variant font-label">
            {t('frag.tapBelowToConnect')} {displayName}. You&apos;ll come back here automatically once it&apos;s
            done.
          </p>
        )}

        <a href={link.linking_url} target="_blank" rel="noreferrer" className="w-full sm:w-auto">
          <Button variant="primary" className="w-full">
            {isDeepLink ? t('frag.openApp', { app: displayName }) : t('frag.connectWithApp', { app: displayName })}
          </Button>
        </a>

        <div className="flex items-center gap-2 text-xs text-on-surface-variant">
          <span className="pierre-spinner w-3.5 h-3.5 border-on-surface border-t-transparent" />
          {t('frag.waitingForYouIn')} {displayName}…
        </div>
      </div>

      <div className="mt-8">
        <Button variant="secondary" onClick={onSkip} className="w-full">
          {t('onboarding.skipForNow')}
        </Button>
      </div>
    </OnboardingShell>
  );
}
