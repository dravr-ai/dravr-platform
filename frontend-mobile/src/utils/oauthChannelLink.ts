// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Links Slack/Discord from the phone: provider authorize page in an auth session, code handed back to the app
// ABOUTME: The app finishes the link with its own login, so a forwarded link can never bind someone else (carnet#1580)

import * as Linking from 'expo-linking';
import * as WebBrowser from 'expo-web-browser';
import type { LinkInitResponse } from '@pierre/api-client';
import { describeApiError, type ApiErrorTranslate } from '@pierre/ui-logic';
import { channelLinkApi } from '../services/api';
import { callbackQuery } from './callbackQuery';

/**
 * The path a Slack or Discord link returns to.
 *
 * `Linking.createURL` turns it into `dravr://messaging/link-callback` in a
 * build and `exp://<host>/--/messaging/link-callback` in Expo Go. The server
 * accepts the first only, and the second only when it runs with
 * `OAUTH_ALLOW_EXPO_GO_REDIRECT=true`. The route
 * `app/messaging/link-callback.tsx` answers the same path, for the
 * platforms that also deliver the redirect to the app as a deep link.
 */
export const CHANNEL_LINK_CALLBACK_PATH = 'messaging/link-callback';

/** Where this app takes back a channel link's provider code; sent with `initLink`. */
export function channelLinkReturnUri(): string {
  return Linking.createURL(CHANNEL_LINK_CALLBACK_PATH);
}

/** The provider's code came back incomplete, or for another link than this one. */
export class ChannelLinkRefusedError extends Error {
  constructor() {
    // A code for logs, never shown: describeChannelLinkError words it.
    super('channel_link_refused');
    this.name = 'ChannelLinkRefusedError';
  }
}

/** The link code a link's authorize URL carries as `state`, which the provider echoes back. */
function linkState(link: LinkInitResponse): string | null {
  return new URLSearchParams(callbackQuery(link.linking_url)).get('state');
}

/**
 * Run an OAuth channel link the app started with {@link channelLinkReturnUri}.
 *
 * Opens the provider's authorize page in an auth session (ASWebAuthenticationSession
 * on iOS, a Custom Tab on Android). The server's callback cannot tell who is
 * holding that browser, so it hands the provider's code back to the app, and the
 * app finishes the link with its own login: the server links it only when that
 * login is the account that started it. The session shares the system browser's
 * cookies, so an athlete already signed in to Slack or Discord there is not asked
 * to sign in again.
 *
 * Resolves `true` once linked, `false` when the athlete closed the sheet or
 * declined on the provider's page. Throws
 * {@link ChannelLinkRefusedError} for a callback that is not this link's, and the
 * API error when the server refuses to finish it.
 */
export async function runOAuthChannelLink(
  link: LinkInitResponse,
  returnUri: string,
): Promise<boolean> {
  const result = await WebBrowser.openAuthSessionAsync(link.linking_url, returnUri);
  if (result.type !== 'success') {
    // cancel / dismiss / locked: the athlete closed the sheet, or another auth
    // session already holds the browser. Nothing to report.
    return false;
  }
  // The return URI exactly, then its query: a longer path that merely starts
  // with it is another address.
  if (result.url !== returnUri && !result.url.startsWith(`${returnUri}?`)) {
    throw new ChannelLinkRefusedError();
  }
  const params = new URLSearchParams(callbackQuery(result.url));
  if (params.get('error') !== null) {
    // The athlete declined on Slack or Discord: the server hands that back so
    // the sheet closes. Nothing was linked and nothing needs saying.
    return false;
  }
  const state = params.get('state');
  const code = params.get('code');
  if (!state || !code || params.get('channel') !== link.channel || state !== linkState(link)) {
    throw new ChannelLinkRefusedError();
  }
  await channelLinkApi.completeLink(link.channel, state, code);
  return true;
}

/**
 * The sentence to show when starting or finishing a channel link failed.
 *
 * A callback that is not this link's is not a network failure, which is how a
 * plain error would otherwise read; a refusal from the server keeps its own
 * sentence.
 */
export function describeChannelLinkError(err: unknown, t: ApiErrorTranslate): string {
  if (err instanceof ChannelLinkRefusedError) {
    return t('app.somethingWentWrongRetry');
  }
  return describeApiError(err, { t, fallbackKey: 'app.failedLoadChannels' });
}
