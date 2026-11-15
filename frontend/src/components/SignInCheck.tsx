// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The /sign-in-check page — a sign-in a provider's robot check parked, answered full-page from its takeover link
// ABOUTME: Opened by the hosted sign-in pages and by the phone app's WebView, which it tells when the sign-in connected or closed

import { useCallback, useMemo, useState } from 'react';
import { useTranslation } from '@pierre/i18n';
import { describeApiError, takeoverTarget, takeoverTokenProvider, type TakeoverResult } from '@pierre/ui-logic';
import { SCIOTTE_LOGIN_PRESETS } from '@pierre/shared-constants';
import type { SciotteTakeoverBridgeMessage, SciotteTakeoverOutcome } from '@pierre/shared-types';
import TakeoverViewer from './takeover/TakeoverViewer';

/** The bridge a React Native WebView puts on the page it loads. */
interface ReactNativeBridge {
  postMessage: (message: string) => void;
}

/** The bridge of the phone app that loaded this page, when one did. */
function appBridge(): ReactNativeBridge | undefined {
  return (window as Window & { ReactNativeWebView?: ReactNativeBridge }).ReactNativeWebView;
}

/** Tell the phone app that loaded this page, when one did, how the takeover ended. */
function tellApp(message: SciotteTakeoverBridgeMessage): void {
  appBridge()?.postMessage(JSON.stringify(message));
}

/** Where the page stands: the takeover runs, it connected, or it is over without connecting. */
type CheckState = { kind: 'running' } | { kind: 'connected' } | { kind: 'closed'; message: string };

interface SignInCheckProps {
  /** The takeover token from the link's fragment, or `null` when it carried none. */
  token: string | null;
}

export default function SignInCheck({ token }: SignInCheckProps) {
  const { t } = useTranslation();
  // The provider is named only inside the token: read for the prompt's words,
  // never trusted — the server verifies the token on the socket.
  const providerLabel = useMemo(() => {
    const provider = token ? takeoverTokenProvider(token, (b64) => window.atob(b64)) : null;
    const target = provider ? takeoverTarget(provider) : null;
    return target ? t(SCIOTTE_LOGIN_PRESETS[target].labelKey) : '';
  }, [token, t]);
  const endedMessage = t('shell.sciotteTakeoverEnded', { provider: providerLabel });
  const [state, setState] = useState<CheckState>(() =>
    token ? { kind: 'running' } : { kind: 'closed', message: endedMessage },
  );

  const onOutcome = useCallback(
    (outcome: SciotteTakeoverOutcome, result: TakeoverResult) => {
      if (result.kind === 'answer' && result.data.status === 'connected') {
        // A sign-in owing its grant goes on to that authorization (Garmin's,
        // which pushes workouts; carnet#1494): the phone app starts it its
        // own way, and a hosted sign-in, whose page is gone, opens the
        // authorization URL the server minted for it.
        const followOn = result.data.oauth_follow_on ?? null;
        setState({ kind: 'connected' });
        if (appBridge()) {
          tellApp(followOn ? { type: 'connected', follow_on: followOn.provider } : { type: 'connected' });
        } else if (followOn?.authorization_url) {
          window.location.assign(followOn.authorization_url);
        }
        return;
      }
      // Anything else ends the takeover here: a page has no code step or
      // second try of its own, and the window that opened it words the answer.
      const message =
        result.kind === 'refusal'
          ? describeApiError(result.error, { t, fallbackKey: 'shell.sciotteLoginFailed' })
          : result.kind === 'answer' && result.data.status === 'failed'
            ? t('auth.loginFailed')
            : t('shell.sciotteTakeoverEnded', { provider: providerLabel });
      setState({ kind: 'closed', message });
      tellApp({ type: 'closed', outcome });
    },
    [providerLabel, t],
  );

  const onEnded = useCallback(() => {
    setState({ kind: 'closed', message: t('shell.sciotteTakeoverEnded', { provider: providerLabel }) });
    tellApp({ type: 'closed' });
  }, [providerLabel, t]);

  return (
    <div className="min-h-dvh flex items-start sm:items-center justify-center py-3 px-3 sm:py-6 sm:px-4 bg-surface pad-safe-top">
      <main className="max-w-xl w-full rounded-xl border ghost-border bg-surface-container-lowest p-3 sm:p-6">
        {state.kind === 'running' && token && (
          <TakeoverViewer
            token={token}
            providerLabel={providerLabel}
            onOutcome={onOutcome}
            onEnded={onEnded}
            viewClassName="h-[max(320px,min(calc(100dvh_-_12rem),760px))]"
          />
        )}
        {state.kind === 'connected' && (
          <div className="py-12 text-center" data-testid="sign-in-check-connected">
            <div className="w-16 h-16 mx-auto mb-4 rounded-full bg-activity/10 flex items-center justify-center">
              <svg className="w-8 h-8 text-activity" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
              </svg>
            </div>
            <h1 className="text-activity text-lg font-medium">{t('shell.sciotteConnected')}</h1>
            {providerLabel && (
              <p className="text-on-surface-variant text-sm mt-1">
                {t('shell.sciotteConnectedDataReady', { provider: providerLabel })}
              </p>
            )}
          </div>
        )}
        {state.kind === 'closed' && (
          <div className="py-12 text-center" data-testid="sign-in-check-closed">
            <div className="w-16 h-16 mx-auto mb-4 rounded-full bg-error/10 flex items-center justify-center">
              <svg className="w-8 h-8 text-error" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L4.082 16.5c-.77.833.192 2.5 1.732 2.5z" />
              </svg>
            </div>
            <p role="alert" className="text-on-surface text-sm max-w-sm mx-auto">{state.message}</p>
          </div>
        )}
      </main>
    </div>
  );
}
