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
    <div
      data-testid="connect-provider-banner"
      className="mb-2 flex items-center gap-3 border-b ghost-border py-2"
    >
      <svg className="w-5 h-5 flex-shrink-0 text-primary" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8.111 16.404a5.5 5.5 0 017.778 0M12 20h.01m-7.08-7.071c3.904-3.905 10.236-3.905 14.141 0M1.394 9.393c5.857-5.857 15.355-5.857 21.213 0" />
      </svg>
      <div className="flex-1 min-w-0">
        <p className="text-sm font-medium text-on-surface">{t('shell.connectBannerTitle')}</p>
        <p className="text-xs text-on-surface-variant">
          {t('shell.connectBannerBody')}
        </p>
      </div>
      <button
        type="button"
        onClick={() => onNavigate(CONNECTIONS_ROUTE)}
        className="btn-base btn-tertiary flex-shrink-0 min-h-[44px] px-2 text-sm"
      >
        {t('shell.connectBannerAction')}
      </button>
      <button
        type="button"
        onClick={() => setDismissed(true)}
        aria-label={t('chat.dismiss')}
        className="flex-shrink-0 inline-flex items-center justify-center min-h-[44px] min-w-[44px] rounded-lg text-on-surface-variant hover:text-on-surface"
      >
        <svg className="w-4 h-4" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>
    </div>
  );
}
