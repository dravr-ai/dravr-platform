// ABOUTME: A provider's own sign-in page in a WebView on the athlete's phone, handing its cookies over once it lands signed in
// ABOUTME: The phone's network and the athlete's own touches sign in, so no cloud browser meets the provider's robot check (carnet#1617)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useCallback, useMemo, useRef } from 'react';
import { View, type ViewStyle } from 'react-native';
import { WebView, type WebViewNavigation } from 'react-native-webview';
import CookieManager, { type Cookie } from '@react-native-cookies/cookies';
import { browserSigninHost, browserSigninHostUnder, browserSigninLanded } from '@pierre/ui-logic';
import type { BrowserSignin, BrowserSigninCookie } from '@pierre/shared-types';

/**
 * The provider's cookies the WebView's store holds for each of `hosts`, merged
 * by name and domain. iOS reads WebKit's store, HttpOnly cookies included;
 * Android's store answers name and value per host, so a cookie it names is
 * given the host it was read for.
 */
export async function readProviderCookies(
  hosts: readonly string[],
  cookieDomain: string,
): Promise<BrowserSigninCookie[]> {
  const merged = new Map<string, BrowserSigninCookie>();
  for (const host of hosts) {
    const jar = await CookieManager.get(`https://${host}/`, true);
    for (const cookie of Object.values(jar) as Cookie[]) {
      const domain = cookie.domain && cookie.domain.length > 0 ? cookie.domain : host;
      if (!browserSigninHostUnder(domain.replace(/^\./, ''), cookieDomain)) continue;
      const expires = cookie.expires ? Date.parse(cookie.expires) / 1000 : undefined;
      merged.set(`${domain}|${cookie.name}`, {
        name: cookie.name,
        value: cookie.value,
        domain,
        path: cookie.path || '/',
        secure: cookie.secure ?? true,
        http_only: cookie.httpOnly ?? false,
        ...(expires !== undefined && Number.isFinite(expires) ? { expires } : {}),
      });
    }
  }
  return [...merged.values()];
}

export interface BrowserSigninWebViewProps {
  /** The provider's entry from `GET /api/providers/browser-signin`. */
  entry: BrowserSignin;
  /** The sign-in landed: the provider's cookies, to post to Dravr. */
  onSignedIn: (cookies: BrowserSigninCookie[]) => void;
  /** The cookies could not be read once signed in. */
  onError: (error: unknown) => void;
  /** The view's height in points. */
  height: number;
  testID?: string;
}

// A WebView takes no className; it fills the View that sizes it.
const webViewStyle: ViewStyle = { flex: 1, backgroundColor: 'transparent' };

export function BrowserSigninWebView({
  entry,
  onSignedIn,
  onError,
  height,
  testID = 'browser-signin-webview',
}: BrowserSigninWebViewProps) {
  const source = useMemo(() => ({ uri: entry.login_url }), [entry.login_url]);
  const frameStyle = useMemo<ViewStyle>(() => ({ height }), [height]);
  // Every host the sign-in passed through, whose cookies make the session.
  const hosts = useRef(new Set<string>([entry.cookie_domain]));
  const handedOver = useRef(false);

  const handleNavigation = useCallback(
    (navigation: WebViewNavigation) => {
      const host = browserSigninHost(navigation.url);
      if (host && browserSigninHostUnder(host, entry.cookie_domain)) hosts.current.add(host);
      if (handedOver.current || !browserSigninLanded(navigation.url, entry)) return;
      handedOver.current = true;
      // The provider's cookies leave the phone's WebView store once read: a
      // second account signing in on this phone must not reuse this session.
      readProviderCookies([...hosts.current], entry.cookie_domain)
        .then(async (cookies) => {
          await CookieManager.clearAll(true);
          return cookies;
        })
        .then(onSignedIn, onError);
    },
    [entry, onError, onSignedIn],
  );

  return (
    <View className="w-full rounded-xl overflow-hidden border border-border" style={frameStyle} testID={`${testID}-frame`}>
      <WebView
        source={source}
        onNavigationStateChange={handleNavigation}
        // The provider's own cookies stay in the store the cookie manager reads.
        sharedCookiesEnabled
        thirdPartyCookiesEnabled
        javaScriptEnabled
        scrollEnabled
        setSupportMultipleWindows={false}
        style={webViewStyle}
        testID={testID}
      />
    </View>
  );
}
