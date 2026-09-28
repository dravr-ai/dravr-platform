// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The bridge starts a provider's OAuth flow through Dravr's connect_provider MCP tool
// ABOUTME: Pins the minted URL it opens, the notice refusal read as a message, and any other refusal reported as one

const {
  startProviderOAuth,
  providerNoticeMessage,
  NOTICE_REQUIRED_ERROR_TYPE,
  PierreError,
} = require('../../dist/index.js');

/** A tool caller that records every call and answers each with `answer`. */
function recordingCaller(answer) {
  const calls = [];
  return {
    calls,
    callTool: async (params) => {
      calls.push(params);
      return answer;
    },
  };
}

const MINTED = 'https://api.prod.whoop.com/oauth/oauth2/auth?client_id=1&state=user-1%3Aflow';

describe('startProviderOAuth', () => {
  test('asks the connect_provider tool for the provider and opens the page it minted', async () => {
    const dravr = recordingCaller({
      content: [{ type: 'text', text: 'pending_authorization' }],
      structuredContent: {
        provider: 'whoop',
        authorization_url: MINTED,
        state: 'user-1:flow',
        status: 'pending_authorization',
      },
      isError: false,
    });

    const start = await startProviderOAuth(dravr.callTool, 'whoop');

    expect(start).toEqual({ kind: 'authorize', url: MINTED });
    // One call, naming only the provider: the athlete is the session's credential.
    expect(dravr.calls).toEqual([{ name: 'connect_provider', arguments: { provider: 'whoop' } }]);
  });

  test('reads the notice refusal as where to accept it, not a raw error', async () => {
    const dravr = recordingCaller({
      content: [{ type: 'text', text: 'notice' }],
      structuredContent: {
        error: 'Connecting WHOOP requires accepting the account notice first',
        error_type: NOTICE_REQUIRED_ERROR_TYPE,
        provider: 'whoop',
      },
      isError: true,
    });

    const start = await startProviderOAuth(dravr.callTool, 'whoop');

    expect(start.kind).toBe('notice_required');
    expect(start.message).toBe(providerNoticeMessage('whoop'));
    expect(start.message).toContain('Open the Dravr app, go to Connections and connect WHOOP');
  });

  test('reports any other refusal with what Dravr said, and opens nothing', async () => {
    const dravr = recordingCaller({
      content: [{ type: 'text', text: 'Provider \'fitbit\' is not supported. Supported providers: strava, whoop' }],
      structuredContent: { error: 'Provider \'fitbit\' is not supported' },
      isError: true,
    });

    const failure = startProviderOAuth(dravr.callTool, 'fitbit');

    await expect(failure).rejects.toBeInstanceOf(PierreError);
    await expect(failure).rejects.toThrow(
      "Dravr refused to start fitbit authorization: Provider 'fitbit' is not supported. Supported providers: strava, whoop",
    );
  });

  test('an answer without an authorization URL is an error, never a page', async () => {
    const dravr = recordingCaller({
      content: [{ type: 'text', text: '{}' }],
      structuredContent: { provider: 'whoop', status: 'pending_authorization' },
      isError: false,
    });

    await expect(startProviderOAuth(dravr.callTool, 'whoop')).rejects.toThrow(
      'Dravr answered the whoop authorization request without an authorization_url',
    );
  });
});
