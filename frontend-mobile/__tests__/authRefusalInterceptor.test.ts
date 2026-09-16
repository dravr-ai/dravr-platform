// ABOUTME: Drives the shared response interceptor for real on the mobile adapter with a stubbed transport
// ABOUTME: Pins which refusal tears the phone's session down — 401 and insufficient_scope, never a role 403

import {
  AxiosError,
  AxiosHeaders,
  type AxiosAdapter,
  type AxiosResponse,
  type InternalAxiosRequestConfig,
} from 'axios';
import {
  createAxiosClient,
  createMobileAdapter,
  type AsyncStorageLike,
} from '@pierre/api-client';

/**
 * RFC 6750 §3.1's challenge for a grant too narrow for the request, exactly as
 * the server sends it: the error code the client must read, alongside the scope
 * to ask for and the RFC 9728 metadata pointer.
 */
const INSUFFICIENT_SCOPE_CHALLENGE =
  'Bearer resource_metadata="https://x/.well-known/oauth-protected-resource", ' +
  'error="insufficient_scope", scope="fitness:write"';

/** The token the session starts holding, so a torn-down session is observable. */
const STORED_TOKEN = 'jwt-for-this-session';

interface SecureStorageLike {
  getItemAsync(key: string): Promise<string | null>;
  setItemAsync(key: string, value: string): Promise<void>;
  deleteItemAsync(key: string): Promise<void>;
}

function fakeAsyncStorage(): AsyncStorageLike {
  const store = new Map<string, string>();
  return {
    getItem: async (key) => store.get(key) ?? null,
    setItem: async (key, value) => {
      store.set(key, value);
    },
    removeItem: async (key) => {
      store.delete(key);
    },
    multiRemove: async (keys) => {
      keys.forEach((key) => store.delete(key));
    },
  };
}

function fakeSecureStore(): SecureStorageLike {
  const store = new Map<string, string>();
  return {
    getItemAsync: async (key) => store.get(key) ?? null,
    setItemAsync: async (key, value) => {
      store.set(key, value);
    },
    deleteItemAsync: async (key) => {
      store.delete(key);
    },
  };
}

interface StubbedReply {
  status: number;
  headers?: Record<string, string>;
  data?: unknown;
}

/**
 * A transport that answers with `reply` and opens no socket.
 *
 * Stubbing axios's adapter rather than mocking the axios module: a module mock
 * replaces `interceptors.response.use`, so the handler under test is never
 * installed and the suite would only prove that a mock was called.
 *
 * A custom adapter settles its own request — axios applies `validateStatus`
 * inside its built-in adapters, not in `dispatchRequest` — so a refusal is
 * raised here as the `AxiosError` the interceptor expects to read.
 */
function stubTransport(reply: StubbedReply): AxiosAdapter {
  return async (config: InternalAxiosRequestConfig) => {
    const response: AxiosResponse = {
      status: reply.status,
      statusText: String(reply.status),
      headers: new AxiosHeaders(reply.headers ?? {}),
      data: reply.data ?? {},
      config,
      request: {},
    };
    if (reply.status >= 200 && reply.status < 300) {
      return response;
    }
    throw new AxiosError(
      `Request failed with status code ${reply.status}`,
      AxiosError.ERR_BAD_REQUEST,
      config,
      {},
      response,
    );
  };
}

async function harness(reply: StubbedReply) {
  const adapter = createMobileAdapter({
    asyncStorage: fakeAsyncStorage(),
    secureStorage: fakeSecureStore(),
    baseURL: 'http://127.0.0.1:8081',
  });
  await adapter.authStorage.setToken(STORED_TOKEN);
  const clear = jest.spyOn(adapter.authStorage, 'clear');
  const signedOut = jest.fn();
  adapter.authFailure.subscribe(signedOut);

  const client = createAxiosClient(adapter);
  client.defaults.adapter = stubTransport(reply);

  return { adapter, client, clear, signedOut };
}

/** The refresh token a signed-in phone holds, and the successor an exchange answers with. */
const STORED_REFRESH_TOKEN = 'refresh-token-1';
const ROTATED_REFRESH_TOKEN = 'refresh-token-2';
const FRESH_TOKEN = 'jwt-after-exchange';

/** What the token endpoint answers when the exchange succeeds. */
const EXCHANGE_OK: StubbedReply = {
  status: 200,
  data: {
    access_token: FRESH_TOKEN,
    token_type: 'Bearer',
    expires_in: 86400,
    refresh_token: ROTATED_REFRESH_TOKEN,
    csrf_token: 'csrf-after-exchange',
    user: { user_id: 'u1', email: 'athlete@example.com' },
  },
};

/** What it answers when the token was revoked, replayed or has expired. */
const EXCHANGE_REFUSED: StubbedReply = {
  status: 400,
  data: { error: 'invalid_grant', error_description: 'Invalid or expired refresh token' },
};

/**
 * A transport that decides per request, so one test can hold a refusal, an
 * exchange and a retry. Records every request it saw, body included, so the
 * test can read what the client sent to the token endpoint.
 */
function scriptedTransport(
  decide: (config: InternalAxiosRequestConfig) => StubbedReply
): { adapter: AxiosAdapter; seen: InternalAxiosRequestConfig[] } {
  const seen: InternalAxiosRequestConfig[] = [];
  const adapter: AxiosAdapter = async (config) => {
    seen.push(config);
    return stubTransport(decide(config))(config);
  };
  return { adapter, seen };
}

/**
 * A harness whose phone also holds a refresh token, against a transport that
 * answers each request from `decide`.
 */
async function refreshHarness(decide: (config: InternalAxiosRequestConfig) => StubbedReply) {
  const adapter = createMobileAdapter({
    asyncStorage: fakeAsyncStorage(),
    secureStorage: fakeSecureStore(),
    baseURL: 'http://127.0.0.1:8081',
  });
  await adapter.authStorage.setToken(STORED_TOKEN);
  await adapter.authStorage.setRefreshToken(STORED_REFRESH_TOKEN);
  const clear = jest.spyOn(adapter.authStorage, 'clear');
  const signedOut = jest.fn();
  adapter.authFailure.subscribe(signedOut);

  const client = createAxiosClient(adapter);
  const transport = scriptedTransport(decide);
  client.defaults.adapter = transport.adapter;

  return { adapter, client, clear, signedOut, seen: transport.seen };
}

const isExchange = (config: InternalAxiosRequestConfig) =>
  config.method === 'post' && config.url === '/oauth/token';

const bearerOf = (config: InternalAxiosRequestConfig) =>
  String(new AxiosHeaders(config.headers).get('Authorization') ?? '');

describe('the refresh-token exchange behind a 401 on the mobile adapter', () => {
  it('exchanges the refresh token, stores the successor and retries with the fresh JWT', async () => {
    const { adapter, client, clear, signedOut, seen } = await refreshHarness((config) => {
      if (isExchange(config)) return EXCHANGE_OK;
      // The retry carries the fresh JWT; the first attempt, the stale one.
      return bearerOf(config) === `Bearer ${FRESH_TOKEN}`
        ? { status: 200, data: { conversations: ['c1'] } }
        : { status: 401, data: { code: 'AuthExpired', message: 'Token expired' } };
    });

    const response = await client.get('/api/chat/conversations');

    expect(response.data).toEqual({ conversations: ['c1'] });
    expect(seen.map((c) => c.url)).toEqual([
      '/api/chat/conversations',
      '/oauth/token',
      '/api/chat/conversations',
    ]);
    // The exchange is the RFC 6749 §6 form, carrying the token the phone held.
    const exchange = seen[1];
    expect(String(exchange.data)).toBe(
      `grant_type=refresh_token&refresh_token=${STORED_REFRESH_TOKEN}`
    );
    // The successor replaced the token just spent; the phone is still signed in.
    expect(await adapter.authStorage.getToken()).toBe(FRESH_TOKEN);
    expect(await adapter.authStorage.getRefreshToken()).toBe(ROTATED_REFRESH_TOKEN);
    expect(await adapter.authStorage.getCsrfToken()).toBe('csrf-after-exchange');
    expect(clear).not.toHaveBeenCalled();
    expect(signedOut).not.toHaveBeenCalled();
  });

  it('signs out when the server refuses the exchange', async () => {
    const { adapter, client, clear, signedOut, seen } = await refreshHarness((config) =>
      isExchange(config)
        ? EXCHANGE_REFUSED
        : { status: 401, data: { code: 'AuthExpired', message: 'Token expired' } }
    );

    await expect(client.get('/api/chat/conversations')).rejects.toMatchObject({
      response: { status: 401 },
    });

    // One attempt, then the session ends — the 400 from the token endpoint
    // never re-enters the interceptor as a refusal to recover from.
    expect(seen.filter(isExchange)).toHaveLength(1);
    expect(clear).toHaveBeenCalledTimes(1);
    expect(signedOut).toHaveBeenCalledTimes(1);
    expect(await adapter.authStorage.getToken()).toBeNull();
    expect(await adapter.authStorage.getRefreshToken()).toBeNull();
  });

  it('runs one exchange for concurrent 401s and retries every request', async () => {
    const { client, clear, signedOut, seen } = await refreshHarness((config) => {
      if (isExchange(config)) return EXCHANGE_OK;
      return bearerOf(config) === `Bearer ${FRESH_TOKEN}`
        ? { status: 200, data: { url: config.url } }
        : { status: 401, data: { code: 'AuthExpired', message: 'Token expired' } };
    });

    // A cold start fires several requests at once; each gets a 401 on the
    // stale JWT. The server revokes a refresh token as it exchanges it, so a
    // second exchange of the same token would read as a replay and kill the
    // session all of them were trying to save.
    const [a, b, c] = await Promise.all([
      client.get('/api/chat/conversations'),
      client.get('/api/user/profile'),
      client.get('/api/notifications'),
    ]);

    expect([a.data, b.data, c.data]).toEqual([
      { url: '/api/chat/conversations' },
      { url: '/api/user/profile' },
      { url: '/api/notifications' },
    ]);
    expect(seen.filter(isExchange)).toHaveLength(1);
    expect(clear).not.toHaveBeenCalled();
    expect(signedOut).not.toHaveBeenCalled();
  });

  it('retries once: a 401 on the fresh JWT ends the session instead of looping', async () => {
    const { client, clear, signedOut, seen } = await refreshHarness((config) =>
      isExchange(config)
        ? EXCHANGE_OK
        : { status: 401, data: { code: 'AuthInvalid', message: 'Account suspended' } }
    );

    await expect(client.get('/api/chat/conversations')).rejects.toMatchObject({
      response: { status: 401 },
    });

    expect(seen.map((c) => c.url)).toEqual([
      '/api/chat/conversations',
      '/oauth/token',
      '/api/chat/conversations',
    ]);
    expect(clear).toHaveBeenCalledTimes(1);
    expect(signedOut).toHaveBeenCalledTimes(1);
  });

  it('never exchanges for a 403, whose grant a re-minted JWT would repeat', async () => {
    const { client, clear, signedOut, seen } = await refreshHarness(() => ({
      status: 403,
      headers: { 'www-authenticate': INSUFFICIENT_SCOPE_CHALLENGE },
      data: { code: 'PermissionDenied', message: 'This token cannot write activities' },
    }));

    await expect(client.post('/api/activities', {})).rejects.toMatchObject({
      response: { status: 403 },
    });

    expect(seen.filter(isExchange)).toHaveLength(0);
    expect(clear).toHaveBeenCalledTimes(1);
    expect(signedOut).toHaveBeenCalledTimes(1);
  });
});

describe('the shared response interceptor on the mobile adapter', () => {
  it('clears the session and signals sign-in on a 401', async () => {
    const { adapter, client, clear, signedOut } = await harness({
      status: 401,
      data: { code: 'AuthRequired', message: 'Missing or invalid token' },
    });

    await expect(client.get('/api/chat/conversations')).rejects.toMatchObject({
      response: { status: 401 },
    });

    expect(clear).toHaveBeenCalledTimes(1);
    expect(signedOut).toHaveBeenCalledTimes(1);
    expect(await adapter.authStorage.getToken()).toBeNull();
  });

  it('clears the session and signals sign-in on a 403 challenging insufficient_scope', async () => {
    const { adapter, client, clear, signedOut } = await harness({
      status: 403,
      headers: { 'www-authenticate': INSUFFICIENT_SCOPE_CHALLENGE },
      data: { code: 'PermissionDenied', message: 'This token cannot write activities' },
    });

    await expect(client.post('/api/activities', {})).rejects.toMatchObject({
      response: { status: 403 },
    });

    expect(clear).toHaveBeenCalledTimes(1);
    expect(signedOut).toHaveBeenCalledTimes(1);
    expect(await adapter.authStorage.getToken()).toBeNull();
  });

  it('leaves the session signed in on a role 403, and still rejects', async () => {
    const { adapter, client, clear, signedOut } = await harness({
      status: 403,
      data: {
        code: 'PermissionDenied',
        message: "Only the conversation's owner can delete it",
      },
    });

    // The rejection still reaches the caller intact — the screen is what tells
    // the athlete what was refused, so the interceptor must not swallow it.
    await expect(client.delete('/api/chat/conversations/abc')).rejects.toMatchObject({
      response: {
        status: 403,
        data: { code: 'PermissionDenied', message: "Only the conversation's owner can delete it" },
      },
    });

    // Signing the athlete out here would strand them in a login loop: the same
    // refusal lands again the moment they come back.
    expect(clear).not.toHaveBeenCalled();
    expect(signedOut).not.toHaveBeenCalled();
    expect(await adapter.authStorage.getToken()).toBe(STORED_TOKEN);
  });

  it('leaves the session signed in on a 403 whose challenge is a different error code', async () => {
    const { adapter, client, clear, signedOut } = await harness({
      status: 403,
      headers: { 'www-authenticate': 'Bearer error="invalid_request"' },
      data: { code: 'PermissionDenied', message: 'Malformed range' },
    });

    await expect(client.get('/api/activities')).rejects.toMatchObject({
      response: { status: 403 },
    });

    expect(clear).not.toHaveBeenCalled();
    expect(signedOut).not.toHaveBeenCalled();
    expect(await adapter.authStorage.getToken()).toBe(STORED_TOKEN);
  });

  it('leaves a successful response alone', async () => {
    const { adapter, client, clear, signedOut } = await harness({
      status: 200,
      data: { conversations: [] },
    });

    const response = await client.get('/api/chat/conversations');

    expect(response.data).toEqual({ conversations: [] });
    expect(clear).not.toHaveBeenCalled();
    expect(signedOut).not.toHaveBeenCalled();
    expect(await adapter.authStorage.getToken()).toBe(STORED_TOKEN);
  });
});
