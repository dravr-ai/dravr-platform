// ABOUTME: A provider robot check's takeover inside the sign-in sheet — the web app's /sign-in-check page in a WebView
// ABOUTME: The page posts connected or closed back over ReactNativeWebView.postMessage; it never leaves its own origin
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useCallback, useMemo } from 'react';
import { View, type ViewStyle } from 'react-native';
import { WebView, type WebViewMessageEvent } from 'react-native-webview';
import type { ShouldStartLoadRequest } from 'react-native-webview/lib/WebViewTypes';
import { parseTakeoverBridgeMessage } from '@pierre/ui-logic';
import type { SciotteTakeoverBridgeMessage } from '@pierre/shared-types';

/**
 * The origin of an absolute http(s) URL, or `null`. RN's URL polyfill does
 * not implement `origin`, hence the pattern rather than `new URL(...)`.
 */
function takeoverOrigin(url: string): string | null {
  const match = /^(https?:\/\/[^/?#]+)/i.exec(url);
  return match ? match[1].toLowerCase() : null;
}

export interface TakeoverWebViewProps {
  /** The `takeover_url` of a `takeover_required` answer: the web app's takeover page. */
  url: string;
  /** What the page says about how the takeover ended. */
  onMessage: (message: SciotteTakeoverBridgeMessage) => void;
  /** The view's height in points; the page's viewport follows it. */
  height: number;
  testID?: string;
}

// A WebView takes no className; it fills the View that sizes it.
const webViewStyle: ViewStyle = { flex: 1, backgroundColor: 'transparent' };

export function TakeoverWebView({ url, onMessage, height, testID = 'sciotte-takeover-webview' }: TakeoverWebViewProps) {
  const origin = useMemo(() => takeoverOrigin(url), [url]);
  const source = useMemo(() => ({ uri: url }), [url]);
  const frameStyle = useMemo<ViewStyle>(() => ({ height }), [height]);

  const handleMessage = useCallback(
    (event: WebViewMessageEvent) => {
      const message = parseTakeoverBridgeMessage(event.nativeEvent.data);
      if (message) onMessage(message);
    },
    [onMessage],
  );

  // The page is the web app's; anything that would take the WebView elsewhere
  // is not followed. The provider's own page is a stream inside it, never a
  // navigation of the WebView.
  const allowLoad = useCallback(
    (request: ShouldStartLoadRequest) => origin !== null && takeoverOrigin(request.url) === origin,
    [origin],
  );

  return (
    <View className="w-full rounded-xl overflow-hidden border border-border" style={frameStyle} testID={`${testID}-frame`}>
      <WebView
        source={source}
        // No originWhitelist: a URL off it is handed to the system browser
        // (Linking.openURL). The default lets every http(s) load reach
        // allowLoad, which keeps the WebView on the page's origin.
        onShouldStartLoadWithRequest={allowLoad}
        onMessage={handleMessage}
        javaScriptEnabled
        // The page scrolls when the sheet is shorter than it: its keyboard
        // button and the bottom of the provider's page stay reachable.
        scrollEnabled
        setSupportMultipleWindows={false}
        style={webViewStyle}
        testID={testID}
      />
    </View>
  );
}
