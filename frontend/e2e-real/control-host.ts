// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Host headers the Maestro control process answers: its own port on loopback, or on the Android emulator's host alias
// ABOUTME: Anything else is refused, so a page in a browser on this machine cannot reach it through DNS rebinding

/**
 * Whether `host` (a request's `Host` header) names the control process as
 * the flows reach it: `127.0.0.1` or `localhost` from the host and the iOS
 * simulator, `10.0.2.2` from the Android emulator, always on `port`.
 *
 * The process listens on 127.0.0.1 only, but a web page on this machine can
 * still reach that socket under a hostname it controls; such a request
 * carries that hostname, never one of these.
 */
export function isControlHost(host: string | undefined, port: number): boolean {
  if (host === undefined) {
    return false;
  }
  return [`127.0.0.1:${port}`, `localhost:${port}`, `10.0.2.2:${port}`].includes(host.toLowerCase());
}
