// ABOUTME: Dismissible one-line nudge to connect a fitness provider — a hairline row, not a boxed card
// ABOUTME: Shown on coach screens when no provider is connected; routes to the connections pane.

import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { providersApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';
import { useTranslation } from '@pierre/i18n';
import { CONNECTIONS_ROUTE } from '../constants/surfaceLayout';

/**
 * Surfaces the "connect a provider" nudge on coach surfaces (the chat flow has its
 * own modal). Hidden once a provider is connected or the user dismisses it for the
 * session. The action navigates through the caller's own router rather than
 * writing `window.location.hash`, so it lands on the one connections route.
 */
export function ConnectProviderBanner({ onNavigate }: { onNavigate: (route: string) => void }) {
  const { t } = useTranslation();
  const [dismissed, setDismissed] = useState(false);
  const { data, isLoading } = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => providersApi.getProvidersStatus(),
  });
  const hasConnectedProvider = data?.providers?.some((p) => p.connected) ?? false;
  // Stay hidden until the providers query answers. `hasConnectedProvider`
  // defaults to false while it is in flight, so rendering on that alone nudges
  // a connected athlete to connect a provider on every coach-screen load.
  if (dismissed || isLoading || hasConnectedProvider) {
    return null;
  }
  return (
    // One line in the caption size — the title, the action, the dismiss — on
    // the open thread only. The second line and the icon went with Boreal
    // v2.1: the empty state and Settings already explain what connecting does.
    <div
      data-testid="connect-provider-banner"
      className="mx-auto flex max-w-[720px] items-center gap-3 border-b ghost-border-faint py-1.5"
    >
      <p className="min-w-0 flex-1 truncate text-xs text-on-surface-variant">{t('shell.connectBannerTitle')}</p>
      <button
        type="button"
        onClick={() => onNavigate(CONNECTIONS_ROUTE)}
        className="btn-base btn-tertiary btn-sm flex-shrink-0 px-1.5"
      >
        {t('shell.connectBannerAction')}
      </button>
      <button
        type="button"
        onClick={() => setDismissed(true)}
        aria-label={t('chat.dismiss')}
        className="inline-flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-lg text-on-surface-variant hover:text-on-surface touch-target"
      >
        <svg className="h-3.5 w-3.5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>
    </div>
  );
}
