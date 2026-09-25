// ABOUTME: Builds the rejection a refused or unreachable request produces on the real axios client
// ABOUTME: Component tests reject with these rather than a bare Error, which no API call ever throws

import { AxiosError, AxiosHeaders } from 'axios';

/**
 * The error a request the server answered with `status` rejects with.
 *
 * Its `message` is axios's own English ("Request failed with status code
 * 403"), exactly as the app sees it, so a test can assert that the athlete
 * reads the catalogue sentence or the server's `data.message` instead.
 */
export function apiRefusal(status: number, data: Record<string, unknown> = {}): AxiosError {
  const headers = new AxiosHeaders();
  return new AxiosError(
    `Request failed with status code ${status}`,
    status >= 500 ? AxiosError.ERR_BAD_RESPONSE : AxiosError.ERR_BAD_REQUEST,
    undefined,
    undefined,
    { status, statusText: String(status), headers, data, config: { headers } },
  );
}

/** The error a request that never reached a server rejects with. */
export function networkFailure(): AxiosError {
  return new AxiosError('Network Error', AxiosError.ERR_NETWORK);
}
