// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E bundles only: the seeded-session link redeems its single-use code at the Maestro control process and signs the app in
// ABOUTME: Metro leaves app/(auth)/e2e/ out of any bundle but a local E2E development one (metro.config.js); the route re-checks the same at runtime

import React, { useEffect, useRef, useState } from 'react';
import { Text, View } from 'react-native';
import { Redirect, useLocalSearchParams } from 'expo-router';
import { authApi, authStorage } from '../../../src/services/api';
import { getApiUrl } from '../../../src/services/apiUrl';
import { useAuth } from '../../../src/contexts/AuthContext';

/**
 * The hosts a seeded-session bundle may talk to: the simulator's loopback and
 * the Android emulator's alias for the host. A bundle pointed anywhere else is
 * talking to a real server, where a link must never be able to swap accounts.
 */
const LOCAL_HOSTS = ['127.0.0.1', 'localhost', '10.0.2.2'];

/** The control process's port on that host (frontend/e2e-real/home-sync-control.ts). */
const CONTROL_PORT = process.env.EXPO_PUBLIC_E2E_CONTROL_PORT ?? '8098';

/**
 * The host of the API this bundle talks to, when it is a local one. Parsed by
 * hand: React Native's URL does not implement `hostname`.
 */
function localApiHost(): string | undefined {
  const host = /^https?:\/\/([^/:?#]+)/i.exec(getApiUrl())?.[1]?.toLowerCase();
  return host !== undefined && LOCAL_HOSTS.includes(host) ? host : undefined;
}

/**
 * The host to redeem a code at, when this bundle may take a seeded session:
 * a development build (`__DEV__`), built for the E2E lanes (the flag Metro
 * inlines), whose API is local. Any one missing and the link is refused.
 */
function seededSessionHost(): string | undefined {
  if (!__DEV__ || process.env.EXPO_PUBLIC_E2E_SEEDED_SESSIONS !== '1') {
    return undefined;
  }
  return localApiHost();
}

/** One query parameter as a string: a repeated one is refused, never guessed at. */
function single(value: string | string[] | undefined): string | undefined {
  return typeof value === 'string' && value !== '' ? value : undefined;
}

/** The pair the control process answers a code with, or undefined when it is not one. */
function tokenPair(body: unknown): { access_token: string; refresh_token: string } | undefined {
  const pair = body as { access_token?: unknown; refresh_token?: unknown } | null;
  return typeof pair?.access_token === 'string' && typeof pair.refresh_token === 'string'
    ? { access_token: pair.access_token, refresh_token: pair.refresh_token }
    : undefined;
}

/**
 * Redeem `code`, store the pair, have the server confirm it, and take the session.
 *
 * Nothing is written while a session already exists on this app, whether the
 * context holds it or only the keychain does: the link signs a signed-out app
 * in, it never swaps one account for another. The code is spent once, even if
 * the effect runs again. The tokens go into the storage the hosted sign-in's
 * code exchange writes them to, the access token is renewed through
 * `GET /api/auth/session` as a cold start renews it — which also proves the
 * server honours it — and the answer is adopted exactly as a sign-in's is.
 * The root layout's guard then routes the account wherever a sign-in would.
 */
function SeededSession({ code, host }: { code: string; host: string }) {
  const { adoptSession, isAuthenticated, isLoading } = useAuth();
  const [failure, setFailure] = useState<string | null>(null);
  const started = useRef(false);

  useEffect(() => {
    if (isLoading || started.current) {
      return;
    }
    started.current = true;
    void (async () => {
      try {
        const stored = (await authStorage.getToken()) ?? (await authStorage.getRefreshToken());
        if (isAuthenticated || stored !== null) {
          setFailure('a session already exists on this app, so the link was refused');
          return;
        }
        const response = await fetch(
          `http://${host}:${CONTROL_PORT}/seeded-session/redeem?code=${encodeURIComponent(code)}`,
          { method: 'POST' },
        );
        const pair = response.ok ? tokenPair(await response.json()) : undefined;
        if (pair === undefined) {
          setFailure(`the control process refused the code (status ${response.status})`);
          return;
        }
        await authStorage.setToken(pair.access_token);
        await authStorage.setRefreshToken(pair.refresh_token);
        const session = await authApi.getSession();
        await adoptSession(session);
      } catch (error) {
        // The status only: the request carried the bearer the code redeemed.
        const status = (error as { response?: { status?: unknown } } | null)?.response?.status;
        setFailure(`the seeded session failed (status ${String(status ?? 'none')})`);
      }
    })();
  }, [code, host, isLoading, isAuthenticated, adoptSession]);

  return <SeededSessionScreen failure={failure} />;
}

/**
 * The route's screen: empty while the session is installed, the reason it
 * was refused otherwise. The reason is a diagnostic for the Maestro helper,
 * which fails naming it; nobody outside an E2E run ever sees this screen.
 */
function SeededSessionScreen({ failure }: { failure: string | null }) {
  return (
    <View testID="e2e-seeded-session">
      {failure ? <Text testID="e2e-seeded-session-failure">{failure}</Text> : null}
    </View>
  );
}

/**
 * `<app>/--/e2e/session?code=…` in an E2E bundle (carnet#1201): the Maestro
 * flows whose subject is not sign-in open the app with a code the control
 * process holds a `pierre-cli user mint-session` pair behind, because the
 * hosted sign-in sheet can stay blank for minutes on the CI iOS simulator.
 * The link carries the code, never a token: Maestro logs every link it opens.
 *
 * A link that signs an app in must not exist in an app anyone installs, nor
 * work against a real server. Metro leaves this directory out of every bundle
 * that is not a development one built with the flag against a local API
 * (metro.config.js); the checks above refuse the link as well, should it ever
 * be bundled without them. The route sits in the `(auth)` group so the root
 * layout's guard lets it run while signed out.
 */
export default function SeededSessionRoute() {
  const params = useLocalSearchParams<{ code?: string | string[] }>();
  const host = seededSessionHost();
  if (host === undefined) {
    return <Redirect href="/(auth)/login" />;
  }
  const code = single(params.code);
  if (code === undefined) {
    return <SeededSessionScreen failure="the link carries no code" />;
  }
  return <SeededSession code={code} host={host} />;
}
