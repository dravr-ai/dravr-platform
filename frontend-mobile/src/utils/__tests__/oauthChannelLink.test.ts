// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the phone's Slack/Discord link: auth session on the authorize URL, code handed back, link finished by the app
// ABOUTME: A closed sheet links nothing; a callback that is not this link's is refused before the server is asked

import * as WebBrowser from 'expo-web-browser';
import type { LinkInitResponse } from '@pierre/api-client';
import { channelLinkApi } from '../../services/api';
import { i18n } from '@pierre/i18n';
import {
  ChannelLinkRefusedError,
  channelLinkReturnUri,
  describeChannelLinkError,
  runOAuthChannelLink,
} from '../oauthChannelLink';

jest.mock('expo-linking', () => ({
  createURL: jest.fn((path: string) => `dravr://${path}`),
}));

const RETURN_URI = 'dravr://messaging/link-callback';
const openAuthSession = WebBrowser.openAuthSessionAsync as jest.Mock;

const LINK: LinkInitResponse = {
  channel: 'slack',
  method: 'oauth',
  code: null,
  linking_url: 'https://slack.com/openid/connect/authorize?state=link-code',
  expires_at: '2030-01-01T00:00:00Z',
  qr_svg: null,
};

function sheetReturns(url: string): void {
  openAuthSession.mockResolvedValueOnce({ type: 'success', url });
}

describe('runOAuthChannelLink', () => {
  let completeLink: jest.SpyInstance;

  beforeEach(() => {
    jest.clearAllMocks();
    completeLink = jest.spyOn(channelLinkApi, 'completeLink').mockResolvedValue({
      status: 'linked',
      channel: 'slack',
      channel_user_id: 'U1',
    });
  });

  it('returns to the app callback the server accepts', () => {
    expect(channelLinkReturnUri()).toBe(RETURN_URI);
  });

  it('authorizes in an auth session and finishes the link with the code handed back', async () => {
    sheetReturns(`${RETURN_URI}?channel=slack&state=link-code&code=provider-code`);

    await expect(runOAuthChannelLink(LINK, RETURN_URI)).resolves.toBe(true);

    expect(openAuthSession).toHaveBeenCalledWith(LINK.linking_url, RETURN_URI);
    expect(completeLink).toHaveBeenCalledWith('slack', 'link-code', 'provider-code');
  });

  it('links nothing when the athlete closes the sheet', async () => {
    openAuthSession.mockResolvedValueOnce({ type: 'cancel' });

    await expect(runOAuthChannelLink(LINK, RETURN_URI)).resolves.toBe(false);
    expect(completeLink).not.toHaveBeenCalled();
  });

  it('links nothing, and says nothing, when the athlete declines on the provider page', async () => {
    sheetReturns(`${RETURN_URI}?channel=slack&state=link-code&error=access_denied`);

    await expect(runOAuthChannelLink(LINK, RETURN_URI)).resolves.toBe(false);
    expect(completeLink).not.toHaveBeenCalled();
  });

  it.each([
    ['another address', 'https://evil.example/?channel=slack&state=link-code&code=c'],
    ['no provider code', `${RETURN_URI}?channel=slack&state=link-code`],
    ['no link code', `${RETURN_URI}?channel=slack&code=c`],
    ['another channel', `${RETURN_URI}?channel=discord&state=link-code&code=c`],
    ['another link code', `${RETURN_URI}?channel=slack&state=other-link&code=c`],
    ['a longer path', `${RETURN_URI}-evil?channel=slack&state=link-code&code=c`],
  ])('refuses a callback with %s before asking the server', async (_case, url) => {
    sheetReturns(url);

    await expect(runOAuthChannelLink(LINK, RETURN_URI)).rejects.toBeInstanceOf(
      ChannelLinkRefusedError,
    );
    expect(completeLink).not.toHaveBeenCalled();
  });

  it('surfaces the server refusing to finish the link', async () => {
    sheetReturns(`${RETURN_URI}?channel=slack&state=link-code&code=provider-code`);
    completeLink.mockRejectedValueOnce(new Error('403'));

    await expect(runOAuthChannelLink(LINK, RETURN_URI)).rejects.toThrow('403');
  });
});

describe('describeChannelLinkError', () => {
  const t = i18n.t.bind(i18n);

  it('words a callback that was not this link as a retry, not as a network failure', () => {
    expect(describeChannelLinkError(new ChannelLinkRefusedError(), t)).toBe(
      i18n.t('app.somethingWentWrongRetry'),
    );
  });
});
