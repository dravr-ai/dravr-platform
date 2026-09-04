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
