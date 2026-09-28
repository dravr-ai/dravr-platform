// ABOUTME: The one provider banner — a warning strip across the app shell while a connection needs reconnecting,
// ABOUTME: and a dismissible one-line nudge on an open thread while no provider is connected; both lead to the connections pane.

import { useState } from 'react';
import { AlertTriangle } from 'lucide-react';
import { useTranslation } from '@pierre/i18n';
import { CONNECTIONS_ROUTE } from '../constants/surfaceLayout';
import { useProviderConnection } from '../hooks/useHome';
import { formatNameList } from './home/homeFormat';
import { Button } from './ui';

/**
 * Which provider state a mount reports.
 *
 * - `reconnect`: mounted once in the app shell, above every tab. A connected
 *   provider flagged `needs_reauth` syncs nothing until the athlete reconnects
 *   it, so the strip names it on every screen and cannot be dismissed.
 * - `connect`: mounted over an open chat thread. It nudges an athlete with no
 *   provider at all and can be dismissed for the session.
 *
 * The two states never hold at once — reconnecting needs a connected
 * provider — so the two mounts never stack.
 */
export type ProviderBannerKind = 'connect' | 'reconnect';

interface ConnectProviderBannerProps {
  kind: ProviderBannerKind;
  onNavigate: (route: string) => void;
}

/**
 * Reads the provider-status query every provider surface shares and says what
 * the athlete has to do about it. The action navigates through the caller's
 * own router rather than writing `window.location.hash`, so it lands on the one
 * connections route. Nothing renders until that query answers, so a healthy
 * athlete is never flashed a prompt while it is in flight.
 */
export function ConnectProviderBanner({ kind, onNavigate }: ConnectProviderBannerProps) {
  const providers = useProviderConnection();
  if (!providers.loaded) {
    return null;
  }
  if (kind === 'reconnect') {
    return providers.needsReconnect.length > 0 ? (
      <ReconnectStrip names={providers.needsReconnect} onNavigate={onNavigate} />
    ) : null;
  }
  return providers.connected ? null : <ConnectNudge onNavigate={onNavigate} />;
}

/**
 * The reconnect strip: the `warning` tint under its bound
 * `on-warning-container` ink (DESIGN.md §2, bound ink), an icon so the hue is
 * not the only signal, and the filled action. `role="alert"` because the
 * athlete's activities have stopped arriving: it is announced once when it
 * appears, and stays until the connection is renewed.
 *
 * It is the first thing in the shell's <main> for an athlete, so on an
 * installed iOS PWA it is what sits under the notch: `pad-safe-top` pads it
 * past the inset, and gives that up when the offline strip or the admin
 * header is already above it.
 */
function ReconnectStrip({ names, onNavigate }: { names: string[]; onNavigate: (route: string) => void }) {
  const { t, language } = useTranslation();
  return (
    <div
      role="alert"
      data-testid="provider-reconnect-banner"
      className="pad-safe-top flex-shrink-0 border-b border-warning/40 bg-warning/15 px-4 py-2.5 md:px-6"
    >
      <div className="flex items-center gap-3">
        <AlertTriangle className="h-5 w-5 flex-shrink-0 text-on-warning-container" aria-hidden="true" />
        <div className="min-w-0 flex-1 text-sm text-on-warning-container">
          <p className="font-semibold">{t('providers.reconnectNeeded')}</p>
          <p>{t('home.activities.reconnect', { providers: formatNameList(names, language) })}</p>
        </div>
        <Button variant="primary" size="sm" className="flex-shrink-0" onClick={() => onNavigate(CONNECTIONS_ROUTE)}>
          {t('providers.reconnect')}
        </Button>
      </div>
    </div>
  );
}

/**
 * One line in the caption size — the title, the action, the dismiss — on the
 * open thread only. The empty state and Settings already explain what
 * connecting does.
 */
function ConnectNudge({ onNavigate }: { onNavigate: (route: string) => void }) {
  const { t } = useTranslation();
  const [dismissed, setDismissed] = useState(false);
  if (dismissed) {
    return null;
  }
  return (
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
