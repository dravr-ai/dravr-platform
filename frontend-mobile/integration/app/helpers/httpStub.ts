// ABOUTME: Stubs both mobile transports — the axios instance and the fetch a chat turn rides — end to end
// ABOUTME: Routes are keyed by "METHOD /path", and every request is recorded for wire-contract assertions

import type { AxiosAdapter, AxiosResponse, InternalAxiosRequestConfig } from 'axios';

import { apiClient } from '../../../src/services/api';

/** One request the mobile client actually put on the wire. */
export interface RecordedRequest {
  /** Upper-case HTTP verb. */
  method: string;
  /** Path as the domain API built it, query string included. */
  url: string;
  /** Request headers, lower-cased. */
  headers: Record<string, string>;
  /** Parsed JSON body, or `undefined` for a body-less request. */
  body: unknown;
}

/** What the stubbed server answers with. */
export interface StubbedResponse {
  /** HTTP status. Defaults to 200; 400+ rejects the way axios does. */
  status?: number;
  /**
   * Response body.
   *
   * A value is handed to axios already decoded. On the fetch transport a
   * string is sent verbatim — which is how an SSE body is stubbed — and
   * anything else is JSON-encoded, matching the single-document answer the
   * server sends for a slash command.
   */
  data: unknown;
  /** Response headers, lower-cased. */
  headers?: Record<string, string>;
}

/**
 * Frame a turn envelope as the body the chat route answers with.
 *
 * One stream carries the whole turn: the stages the pipeline worked through,
 * any prose deltas, each reply block, then the terminal envelope. Deltas are
 * optional because only the ACP provider branch produces any; progress and
 * blocks are optional here only so a spec can assert what happens without
 * them.
 */
export function sseTurn(
  turn: unknown,
  deltas: string[] = [],
  options: { progress?: unknown[]; blocks?: unknown[] } = {},
): string {
  const { progress = [], blocks = [] } = options;
  return [
    ...progress.map((p) => `event: progress\ndata: ${JSON.stringify(p)}\n\n`),
    ...deltas.map((delta) => `event: delta\ndata: ${JSON.stringify({ delta })}\n\n`),
    ...blocks.map((block) => `event: block\ndata: ${JSON.stringify(block)}\n\n`),
    `event: done\ndata: ${JSON.stringify(turn)}\n\n`,
  ].join('');
}

/** The stage sequence a real turn emits, in pipeline order. */
export const STAGE_PROGRESS = [
  { kind: 'stage', id: 'prompt_assembly', title: 'prompt_assembly', status: 'started' },
  { kind: 'stage', id: 'prompt_assembly', title: 'prompt_assembly', status: 'finished' },
  { kind: 'stage', id: 'dispatch', title: 'dispatch', status: 'started' },
];

/** A route answers with a fixed response, or computes one per request. */
export type StubRoute = StubbedResponse | ((request: RecordedRequest) => StubbedResponse);

/** Routes keyed by `"GET /api/coaches/abc/versions?limit=50"`. */
export type StubRoutes = Record<string, StubRoute>;

/** Handle returned by {@link installHttpStub}. */
export interface HttpStub {
  /** Every request the client made, in order, across both transports. */
  requests: RecordedRequest[];
  /** Requests matching a verb, for narrower assertions. */
  requestsFor(method: string): RecordedRequest[];
  /** Put the original transports back. */
  restore(): void;
}

function headersOf(config: InternalAxiosRequestConfig): Record<string, string> {
  const raw = config.headers as unknown;
  const source =
    raw && typeof (raw as { toJSON?: unknown }).toJSON === 'function'
      ? ((raw as { toJSON: () => Record<string, unknown> }).toJSON())
      : ((raw ?? {}) as Record<string, unknown>);

  const flat: Record<string, string> = {};
  for (const [key, value] of Object.entries(source)) {
    if (value !== undefined && value !== null && typeof value !== 'object') {
      flat[key.toLowerCase()] = String(value);
    }
  }
  return flat;
}

function bodyOf(config: InternalAxiosRequestConfig): unknown {
  if (typeof config.data !== 'string') {
    return config.data;
  }
  try {
    return JSON.parse(config.data) as unknown;
  } catch {
    return config.data;
  }
}

/**
 * Point the app's axios instance at an in-process server.
 *
 * The interceptors, the domain methods and the endpoint constants all still
 * run — only the socket is replaced — so a spec fails when the client stops
 * calling the URL the server serves, or stops reading a field the server
 * sends. An unrouted request throws with the URL that was asked for, which
 * turns a silently-changed endpoint into a named failure.
 */
export function installHttpStub(routes: StubRoutes): HttpStub {
  const requests: RecordedRequest[] = [];
  const previousAdapter = apiClient.defaults.adapter;

  const adapter: AxiosAdapter = async (config) => {
    const request: RecordedRequest = {
      method: (config.method ?? 'get').toUpperCase(),
      url: config.url ?? '',
      headers: headersOf(config),
      body: bodyOf(config),
    };
    requests.push(request);

    const key = `${request.method} ${request.url}`;
    const route = routes[key];
    if (!route) {
      throw new Error(
        `Unstubbed request: ${key}\nStubbed routes: ${Object.keys(routes).join(', ') || '(none)'}`
      );
    }

    const stubbed = typeof route === 'function' ? route(request) : route;
    const status = stubbed.status ?? 200;
    const response: AxiosResponse = {
      data: stubbed.data,
      status,
      statusText: status === 200 ? 'OK' : String(status),
      headers: stubbed.headers ?? {},
      config,
      request: {},
    };

    if (status >= 400) {
      throw Object.assign(new Error(`Request failed with status code ${status}`), {
        isAxiosError: true,
        config,
        response,
      });
    }
    return response;
  };

  apiClient.defaults.adapter = adapter;

  // The chat turn is the one request that does not ride axios: it reads its
  // response body frame by frame, which axios cannot do. Stub it at the same
  // route table so a spec describes the server once.
  const baseUrl = apiClient.defaults.baseURL ?? '';
  const previousFetch = globalThis.fetch;
  const stubbedFetch: typeof fetch = async (input, init) => {
    const rawUrl = typeof input === 'string' ? input : String(input);
    const url = rawUrl.startsWith(baseUrl) ? rawUrl.slice(baseUrl.length) : rawUrl;
    const rawHeaders = (init?.headers ?? {}) as Record<string, string>;
    const headers: Record<string, string> = {};
    for (const [key, value] of Object.entries(rawHeaders)) {
      headers[key.toLowerCase()] = String(value);
    }
    const request: RecordedRequest = {
      method: (init?.method ?? 'GET').toUpperCase(),
      url,
      headers,
      body: typeof init?.body === 'string' ? (JSON.parse(init.body) as unknown) : init?.body,
    };
    requests.push(request);

    const key = `${request.method} ${request.url}`;
    const route = routes[key];
    if (!route) {
      throw new Error(
        `Unstubbed request: ${key}\nStubbed routes: ${Object.keys(routes).join(', ') || '(none)'}`
      );
    }
    const stubbed = typeof route === 'function' ? route(request) : route;
    const body =
      typeof stubbed.data === 'string' ? stubbed.data : JSON.stringify(stubbed.data);
    return new Response(body, {
      status: stubbed.status ?? 200,
      headers: stubbed.headers ?? {},
    });
  };
  globalThis.fetch = stubbedFetch;

  return {
    requests,
    requestsFor: (method: string) =>
      requests.filter((request) => request.method === method.toUpperCase()),
    restore: () => {
      apiClient.defaults.adapter = previousAdapter;
      globalThis.fetch = previousFetch;
    },
  };
}
