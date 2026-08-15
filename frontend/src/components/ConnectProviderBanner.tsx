// ABOUTME: Dismissible banner nudging the user to connect a fitness provider.
// ABOUTME: Shown on coach screens when no provider is connected; routes to the Data Providers tab.

import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { providersApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';

/**
 * Surfaces the "connect a provider" nudge on coach surfaces (the chat flow has its
 * own modal). Hidden once a provider is connected or the user dismisses it for the
 * session. "Connect" deep-links to the top-level Data Providers tab.
 */
export function ConnectProviderBanner() {
  const [dismissed, setDismissed] = useState(false);
  const { data } = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => providersApi.getProvidersStatus(),
  });
  const hasConnectedProvider = data?.providers?.some((p) => p.connected) ?? false;
  if (dismissed || hasConnectedProvider) {
    return null;
  }
  return (
    <div
      data-testid="connect-provider-banner"
      className="mb-4 flex items-center gap-3 rounded-xl border border-primary/30 bg-primary/10 px-4 py-3"
    >
      <svg className="w-5 h-5 flex-shrink-0 text-primary" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8.111 16.404a5.5 5.5 0 017.778 0M12 20h.01m-7.08-7.071c3.904-3.905 10.236-3.905 14.141 0M1.394 9.393c5.857-5.857 15.355-5.857 21.213 0" />
      </svg>
      <div className="flex-1 min-w-0">
        <p className="text-sm font-medium text-on-surface">Connect a fitness provider</p>
        <p className="text-xs text-on-surface-variant">
          Your coach needs your activity data to give real guidance instead of generic advice.
        </p>
      </div>
      <button
        type="button"
        onClick={() => {
          window.location.hash = 'data-providers';
        }}
        className="btn-base btn-primary flex-shrink-0 min-h-[44px] px-4 text-sm"
      >
        Connect
      </button>
      <button
        type="button"
        onClick={() => setDismissed(true)}
        aria-label="Dismiss"
        className="flex-shrink-0 inline-flex items-center justify-center min-h-[44px] min-w-[44px] rounded-lg text-on-surface-variant hover:text-on-surface"
      >
        <svg className="w-4 h-4" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>
    </div>
  );
}
