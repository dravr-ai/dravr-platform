// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests that a stored OAuth client registration Dravr refuses is replaced, once
// ABOUTME: Covers the scope-set check before authorizing, the single retry, and startup cleanup

const { PierreMcpClient } = require('../../dist/index.js');
const {
  closeServer,
  httpRequest,
  startPierreStub,
  startProvider,
  stopProvider,
} = require('./oauth-callback-harness.js');

/** The four delegable scopes the SDK registers and authorizes for. */
const CURRENT_SCOPE = 'fitness:read fitness:write profile:read profile:write';

/** What an earlier SDK release registered with: names outside the server's vocabulary. */
const LEGACY_SCOPE = 'read:fitness write:fitness';

/** The command line's default: no configured client, so the SDK registers its own. */
const DYNAMIC_REGISTRATION = { oauthClientId: '', oauthClientSecret: '' };

const issuedTokens = {
  body: {
    access_token: 'issued-jwt',
    token_type: 'Bearer',
    expires_in: 3600,
    refresh_token: 'issued-refresh',
    scope: CURRENT_SCOPE,
  },
};

function registration(clientId, scope) {
  return {
    client_id: clientId,
    client_secret: `${clientId}-secret`,
    redirect_uris: ['http://localhost:35535/oauth/callback'],
    grant_types: ['authorization_code'],
    response_types: ['code'],
    scope,
    client_name: 'Dravr MCP Client',
  };
}

/** Records what would have gone into the OS keychain, without touching it. */
function recordingStorage(writes) {
  return {
    saveTokens: async (tokens) => {
      writes.push(JSON.parse(JSON.stringify(tokens)));
    },
    getTokens: async () => null,
    clearTokens: async () => {},
  };
}

/** Dravr's registration endpoint: every call is a new client with the server's own id. */
function registerRoute(callNumber) {
  return {
    status: 201,
    body: {
      client_id: `fresh-client-${callNumber}`,
      client_secret: `fresh-secret-${callNumber}`,
      scope: CURRENT_SCOPE,
    },
  };
}

function requestsTo(pierre, path) {
  return pierre.requests.filter((request) => request.url === path);
}

function formField(request, name) {
  return new URLSearchParams(request.body).get(name);
}

/**
 * A sign-in through the bridge against a Dravr stub, with the browser step reduced to
 * what the browser does: each authorization is answered on the callback URL by the next
 * entry of `answers`, a function of the flow's state returning the query strings the
 * browser delivers, in order. The connection after the sign-in is a client reporting one
 * tool, so the test stays on the authorization.
 */
async function signIn({ routes, stored, config = DYNAMIC_REGISTRATION, answers }) {
  const pierre = await startPierreStub(routes);
  const { provider, port } = await startProvider(
    { ...config, authorizationTimeoutMs: 2000 },
    pierre.url,
  );
  const writes = [];
  provider.secureStorage = recordingStorage(writes);
  if (stored) {
    provider.clientInfo = stored;
    provider.allStoredTokens = { client_info: stored };
  }

  const authorizations = [];
  provider.redirectToAuthorization = async (authUrl) => {
    authorizations.push({
      url: authUrl,
      registrationsSoFar: requestsTo(pierre, '/oauth2/register').length,
    });
    await provider.startCallbackServer();
    const completing = provider.completeAuthorization().then(
      () => null,
      (error) => error,
    );
    for (const query of answers[authorizations.length - 1](provider.stateValue)) {
      await httpRequest(port, { path: `/oauth/callback?${query}` });
    }
    const error = await completing;
    if (error) {
      throw error;
    }
  };

  const bridge = new PierreMcpClient({
    mode: 'oauth',
    pierreServerUrl: pierre.url,
    ...config,
  });
  bridge.log = () => {};
  bridge.oauthProvider = provider;
  bridge.attemptConnection = async () => {
    bridge.pierreClient = {
      listTools: async () => ({ tools: [{ name: 'get_activities' }] }),
    };
  };

  const outcome = await bridge.initiateConnection().then(
    () => null,
    (error) => error,
  );

  return { pierre, provider, writes, authorizations, outcome };
}

const approve = (state) => [`code=authorization-code&state=${state}`];
const refuse = (error) => (state) => [`error=${error}&state=${state}`];

async function cleanup({ pierre, provider }) {
  stopProvider(provider);
  await closeServer(pierre.server);
}

describe('a stored registration for another scope set', () => {
  test('is replaced before the first authorization request names it', async () => {
    const run = await signIn({
      routes: { '/oauth2/register': registerRoute, '/oauth2/token': () => issuedTokens },
      stored: registration('legacy-client', LEGACY_SCOPE),
      answers: [approve],
    });

    try {
      expect(run.outcome).toBeNull();

      // One registration, asking for the current scope set, and it went out before the
      // browser was ever sent anywhere.
      const registrations = requestsTo(run.pierre, '/oauth2/register');
      expect(registrations).toHaveLength(1);
      expect(JSON.parse(registrations[0].body).scope).toBe(CURRENT_SCOPE);
      expect(run.authorizations).toHaveLength(1);
      expect(run.authorizations[0].registrationsSoFar).toBe(1);

      // The authorization and the code exchange both present the client Dravr assigned.
      expect(run.authorizations[0].url.searchParams.get('client_id')).toBe('fresh-client-1');
      expect(run.authorizations[0].url.searchParams.get('scope')).toBe(CURRENT_SCOPE);
      const exchanges = requestsTo(run.pierre, '/oauth2/token');
      expect(exchanges).toHaveLength(1);
      expect(formField(exchanges[0], 'client_id')).toBe('fresh-client-1');
      expect(formField(exchanges[0], 'client_secret')).toBe('fresh-secret-1');

      // The keychain ends up holding the new registration and the session issued to it.
      const stored = run.writes[run.writes.length - 1];
      expect(stored.client_info.client_id).toBe('fresh-client-1');
      expect(stored.client_info.scope).toBe(CURRENT_SCOPE);
      expect(stored.pierre.access_token).toBe('issued-jwt');
    } finally {
      await cleanup(run);
    }
  });

  test('is discarded at startup with the session issued to it, without asking the server', async () => {
    const pierre = await startPierreStub({});
    const { provider } = await startProvider(DYNAMIC_REGISTRATION, pierre.url);
    const writes = [];
    provider.secureStorage = recordingStorage(writes);
    const legacy = registration('legacy-client', LEGACY_SCOPE);
    const legacySession = {
      access_token: 'legacy-access',
      refresh_token: 'legacy-refresh',
      token_type: 'Bearer',
      scope: LEGACY_SCOPE,
    };
    provider.allStoredTokens = {
      client_info: legacy,
      pierre: legacySession,
      providers: { strava: { access_token: 'strava-access' } },
    };
    provider.clientInfo = legacy;
    provider.savedTokens = { ...legacySession };

    try {
      await provider.validateAndCleanupCachedCredentials();

      // Nothing about the dead registration or its session went to the server.
      expect(pierre.requests).toHaveLength(0);
      expect(await provider.clientInformation()).toBeUndefined();
      expect(await provider.tokens()).toBeUndefined();

      // Storage lost both, and kept the provider tokens that belong to neither.
      expect(writes).toHaveLength(1);
      expect(writes[0].client_info).toBeUndefined();
      expect(writes[0].pierre).toBeUndefined();
      expect(writes[0].providers.strava.access_token).toBe('strava-access');
    } finally {
      stopProvider(provider);
      await closeServer(pierre.server);
    }
  });
});

describe('a registration for the current scope set', () => {
  test('is kept at startup and validated with the server under its own client id', async () => {
    const pierre = await startPierreStub({
      '/oauth2/token-validate': () => ({ body: { valid: true } }),
    });
    const { provider } = await startProvider(DYNAMIC_REGISTRATION, pierre.url);
    const writes = [];
    provider.secureStorage = recordingStorage(writes);
    const current = registration('stored-client', 'profile:write fitness:read profile:read fitness:write');
    provider.allStoredTokens = { client_info: current };
    provider.clientInfo = current;

    try {
      await provider.validateAndCleanupCachedCredentials();

      // Scope order carries no meaning, so the reordered set is the current one.
      const validations = requestsTo(pierre, '/oauth2/token-validate');
      expect(validations).toHaveLength(1);
      expect(JSON.parse(validations[0].body).client_id).toBe('stored-client');
      expect((await provider.clientInformation()).client_id).toBe('stored-client');
      expect(writes).toHaveLength(0);
    } finally {
      stopProvider(provider);
      await closeServer(pierre.server);
    }
  });
});

describe('a stored registration Dravr refuses', () => {
  test('an invalid_scope answer re-registers exactly once and the retry signs in', async () => {
    const run = await signIn({
      routes: { '/oauth2/register': registerRoute, '/oauth2/token': () => issuedTokens },
      stored: registration('stored-client', CURRENT_SCOPE),
      answers: [refuse('invalid_scope'), approve],
    });

    try {
      expect(run.outcome).toBeNull();
      expect(requestsTo(run.pierre, '/oauth2/register')).toHaveLength(1);

      // First as the stored client, which was refused; then as the one just registered.
      expect(run.authorizations.map((a) => a.url.searchParams.get('client_id'))).toEqual([
        'stored-client',
        'fresh-client-1',
      ]);
      expect(run.authorizations[1].registrationsSoFar).toBe(1);

      const exchanges = requestsTo(run.pierre, '/oauth2/token');
      expect(exchanges).toHaveLength(1);
      expect(formField(exchanges[0], 'client_id')).toBe('fresh-client-1');

      expect((await run.provider.tokens()).access_token).toBe('issued-jwt');
      const stored = run.writes[run.writes.length - 1];
      expect(stored.client_info.client_id).toBe('fresh-client-1');
      expect(stored.pierre.access_token).toBe('issued-jwt');
    } finally {
      await cleanup(run);
    }
  });

  test('a second refusal reaches the caller instead of registering again', async () => {
    const run = await signIn({
      routes: { '/oauth2/register': registerRoute, '/oauth2/token': () => issuedTokens },
      stored: registration('stored-client', CURRENT_SCOPE),
      answers: [refuse('invalid_scope'), refuse('invalid_scope')],
    });

    try {
      expect(run.outcome).not.toBeNull();
      expect(run.outcome.code).toBe('AUTH_ERROR');
      expect(run.outcome.oauthError).toBe('invalid_scope');

      expect(requestsTo(run.pierre, '/oauth2/register')).toHaveLength(1);
      expect(run.authorizations).toHaveLength(2);
      expect(requestsTo(run.pierre, '/oauth2/token')).toHaveLength(0);
      expect(await run.provider.tokens()).toBeUndefined();
    } finally {
      await cleanup(run);
    }
  });

  test('an invalid_client token exchange re-registers once and exchanges as the new client', async () => {
    const run = await signIn({
      routes: {
        '/oauth2/register': registerRoute,
        '/oauth2/token': (callNumber) =>
          callNumber === 1
            ? { status: 400, body: { error: 'invalid_client', error_description: 'expired' } }
            : issuedTokens,
      },
      stored: registration('stored-client', CURRENT_SCOPE),
      answers: [approve, approve],
    });

    try {
      expect(run.outcome).toBeNull();
      expect(requestsTo(run.pierre, '/oauth2/register')).toHaveLength(1);

      const exchanges = requestsTo(run.pierre, '/oauth2/token');
      expect(exchanges.map((request) => formField(request, 'client_id'))).toEqual([
        'stored-client',
        'fresh-client-1',
      ]);
      expect((await run.provider.tokens()).access_token).toBe('issued-jwt');
    } finally {
      await cleanup(run);
    }
  });

  test('a refusal that does not name the registration is not answered with one', async () => {
    const run = await signIn({
      routes: { '/oauth2/register': registerRoute, '/oauth2/token': () => issuedTokens },
      stored: registration('stored-client', CURRENT_SCOPE),
      answers: [refuse('access_denied')],
    });

    try {
      expect(run.outcome.oauthError).toBe('access_denied');
      expect(requestsTo(run.pierre, '/oauth2/register')).toHaveLength(0);
      expect(run.authorizations).toHaveLength(1);
      expect((await run.provider.clientInformation()).client_id).toBe('stored-client');
    } finally {
      await cleanup(run);
    }
  });
});

describe('registrations that are not replaced', () => {
  test('a refusal of the registration made for this sign-in reaches the caller', async () => {
    const run = await signIn({
      routes: { '/oauth2/register': registerRoute, '/oauth2/token': () => issuedTokens },
      answers: [refuse('invalid_scope')],
    });

    try {
      expect(run.outcome.oauthError).toBe('invalid_scope');
      expect(requestsTo(run.pierre, '/oauth2/register')).toHaveLength(1);
      expect(run.authorizations).toHaveLength(1);
    } finally {
      await cleanup(run);
    }
  });

  test('a configured client is presented as given and never registered over', async () => {
    const run = await signIn({
      routes: { '/oauth2/register': registerRoute, '/oauth2/token': () => issuedTokens },
      config: { oauthClientId: 'configured-client', oauthClientSecret: 'configured-secret' },
      answers: [refuse('invalid_client')],
    });

    try {
      expect(run.outcome.oauthError).toBe('invalid_client');
      expect(requestsTo(run.pierre, '/oauth2/register')).toHaveLength(0);
      expect(run.authorizations).toHaveLength(1);
      expect(run.authorizations[0].url.searchParams.get('client_id')).toBe('configured-client');
    } finally {
      await cleanup(run);
    }
  });
});

describe('an authorization error on the callback', () => {
  test('without this flow\'s state it is ignored and the sign-in completes', async () => {
    const run = await signIn({
      routes: { '/oauth2/register': registerRoute, '/oauth2/token': () => issuedTokens },
      stored: registration('stored-client', CURRENT_SCOPE),
      answers: [
        (state) => [
          'error=invalid_scope&state=forged-state',
          'error=invalid_scope',
          `code=authorization-code&state=${state}`,
        ],
      ],
    });

    try {
      expect(run.outcome).toBeNull();
      expect(requestsTo(run.pierre, '/oauth2/register')).toHaveLength(0);
      expect(run.authorizations).toHaveLength(1);
      expect((await run.provider.tokens()).access_token).toBe('issued-jwt');
    } finally {
      await cleanup(run);
    }
  });
});
