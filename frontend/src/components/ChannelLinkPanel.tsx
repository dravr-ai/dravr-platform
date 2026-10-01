// ABOUTME: The body of a chat-app link in progress — QR + deep-link button (Telegram/WhatsApp) or an OAuth connect button
// ABOUTME: Shared by the onboarding connect step and Settings → Messaging, so both hand the athlete the same link

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import type { LinkInitResponse } from '@pierre/api-client';
import { useTranslation } from '@pierre/i18n';
import { Button } from './ui';

/**
 * Render a started channel link.
 *
 * Deep-link channels show a QR — the primary path on desktop, where the athlete
 * runs the chat app on their phone — plus a tap button (the primary path on a
 * phone). OAuth channels show a single connect button that redirects. The
 * caller owns the wait: it polls the linked channels and moves on when this
 * one appears, which is why the panel ends on a "waiting" line.
 */
export default function ChannelLinkPanel({
  link,
  displayName,
}: {
  link: LinkInitResponse;
  displayName: string;
}) {
  const { t } = useTranslation();
  const isDeepLink = link.method === 'deep_link';

  return (
    <div className="flex flex-col items-center gap-5" data-testid="channel-link-panel">
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
          <p className="max-w-sm text-center text-sm text-on-surface-variant">
            {t('onboarding.messagingScanPrefix', { app: displayName })}
            <span className="font-semibold"> {t('onboarding.messagingStartButton')}</span> {t('onboarding.messagingScanSuffix')}
          </p>
        </>
      ) : (
        <p className="max-w-sm text-center text-sm text-on-surface-variant">
          {t('app.tapToConnectAutoReturn', { channel: displayName })}
        </p>
      )}

      <a href={link.linking_url} target="_blank" rel="noreferrer" className="w-full sm:w-auto">
        <Button variant="primary" className="w-full">
          {isDeepLink
            ? t('app.openChannel', { channel: displayName })
            : t('app.connectWithChannel', { channel: displayName })}
        </Button>
      </a>

      <div className="flex items-center gap-2 text-xs text-on-surface-variant">
        <span className="pierre-spinner w-3.5 h-3.5 border-on-surface border-t-transparent" />
        {t('app.waitingToFinishIn', { channel: displayName })}
      </div>
    </div>
  );
}
