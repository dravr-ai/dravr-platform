// ABOUTME: Cloudflare Turnstile bot check for the register form, rendered in a WebView on the API's origin
// ABOUTME: The page posts the token back over ReactNativeWebView.postMessage; reset() asks for a fresh one
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { forwardRef, useCallback, useImperativeHandle, useMemo, useRef } from 'react';
import { View, type ViewStyle } from 'react-native';
import { WebView, type WebViewMessageEvent } from 'react-native-webview';
import { getApiUrl } from '../../services/apiUrl';

/** The explicit-render script; `onload` names the page function that renders the widget. */
const TURNSTILE_SCRIPT_URL =
  'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit&onload=onTurnstileLoad';

/** What the page posts back. */
type TurnstileMessage =
  | { type: 'token'; token: string }
  | { type: 'expired' }
  | { type: 'error' }
  | { type: 'load-error' };

function parseMessage(data: string): TurnstileMessage | null {
  try {
    const parsed: unknown = JSON.parse(data);
    if (typeof parsed !== 'object' || parsed === null || !('type' in parsed)) return null;
    const message = parsed as { type: unknown; token?: unknown };
    if (message.type === 'token') {
      return typeof message.token === 'string' ? { type: 'token', token: message.token } : null;
    }
    if (message.type === 'expired' || message.type === 'error' || message.type === 'load-error') {
      return { type: message.type };
    }
    return null;
  } catch {
    return null;
  }
}

/**
 * The origin the page is served as.
 *
 * Turnstile checks the page's hostname against the site key's allowed
 * domains, so the inline page claims the API's own origin, never
 * `about:blank`. RN's URL polyfill does not implement `origin`, hence the
 * pattern rather than `new URL(...)`.
 */
export function turnstileBaseUrl(apiUrl: string): string {
  const match = /^(https?:\/\/[^/?#]+)/i.exec(apiUrl);
  return match ? `${match[1]}/` : apiUrl;
}

/** The inline page: loads the script once, renders one widget, posts each outcome. */
export function turnstilePageHtml(siteKey: string): string {
  return `<!DOCTYPE html>
<html>
<head>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>html,body{margin:0;padding:0;background:transparent;}body{display:flex;justify-content:center;}</style>
<script>
function post(message){window.ReactNativeWebView.postMessage(JSON.stringify(message));}
window.onTurnstileLoad=function(){
  window.turnstile.render('#turnstile',{
    sitekey:${JSON.stringify(siteKey)},
    theme:'auto',
    callback:function(token){post({type:'token',token:token});},
    'expired-callback':function(){post({type:'expired'});},
    'error-callback':function(){post({type:'error'});}
  });
};
</script>
<script src="${TURNSTILE_SCRIPT_URL}" async defer onerror="post({type:'load-error'})"></script>
</head>
<body><div id="turnstile"></div></body>
</html>`;
}

export interface TurnstileWebViewHandle {
  /** Discard the current token and ask for a new challenge: a token is single-use. */
  reset: () => void;
}

export interface TurnstileWebViewProps {
  siteKey: string;
  /** The token once the visitor passes, `null` once it expires or the widget errors. */
  onToken: (token: string | null) => void;
  /** The script could not be loaded (offline, or blocked). */
  onLoadError?: () => void;
  testID?: string;
}

// A WebView takes no className; it fills the View that sizes it.
const webViewStyle: ViewStyle = { flex: 1, backgroundColor: 'transparent' };

export const TurnstileWebView = forwardRef<TurnstileWebViewHandle, TurnstileWebViewProps>(
  function TurnstileWebView({ siteKey, onToken, onLoadError, testID = 'turnstile-webview' }, ref) {
    const webViewRef = useRef<WebView>(null);
    const source = useMemo(
      () => ({ html: turnstilePageHtml(siteKey), baseUrl: turnstileBaseUrl(getApiUrl()) }),
      [siteKey],
    );

    useImperativeHandle(ref, () => ({
      reset: () => {
        onToken(null);
        webViewRef.current?.injectJavaScript('window.turnstile && window.turnstile.reset(); true;');
      },
    }), [onToken]);

    const handleMessage = useCallback((event: WebViewMessageEvent) => {
      const message = parseMessage(event.nativeEvent.data);
      if (!message) return;
      if (message.type === 'token') {
        onToken(message.token);
      } else if (message.type === 'load-error') {
        onToken(null);
        onLoadError?.();
      } else {
        onToken(null);
      }
    }, [onToken, onLoadError]);

    return (
      <View className="h-[72px] w-full" testID={`${testID}-frame`}>
        <WebView
          ref={webViewRef}
          originWhitelist={['*']}
          source={source}
          onMessage={handleMessage}
          javaScriptEnabled
          scrollEnabled={false}
          style={webViewStyle}
          testID={testID}
        />
      </View>
    );
  },
);
