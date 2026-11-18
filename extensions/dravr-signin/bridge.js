// ABOUTME: The content script on app.dravr.ai: relays the page's window messages to a port into the extension and back
// ABOUTME: One channel for every browser — Firefox and Safari have no externally_connectable (carnet#1617)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// A classic script, as every browser loads a content script: no import, no
// export. bridge.test.js runs this source with a fake window and runtime.
//
// Relay between the page (`window.postMessage`, on the page's own origin) and
// the extension's background (a `runtime` port). The page names each exchange
// by a session id:
// - `{channel, to: 'extension', session, kind: 'open'}` opens a port and is
//   answered `{channel, to: 'page', session, kind: 'opened'}`;
// - `kind: 'message'` carries `payload` onto that port, and each message the
//   background posts comes back as `kind: 'message'` with its `payload`;
// - `kind: 'close'` closes the port; a port the background closes comes back
//   as `kind: 'closed'`.
// Only messages from this window, on its own origin, are relayed: another
// frame or a cross-origin window posting to it is ignored.
(() => {
  const CHANNEL = 'dravr-signin';
  const api = globalThis.browser ?? globalThis.chrome;
  if (typeof window === 'undefined' || !api?.runtime?.connect) return;

  const ports = new Map();
  const toPage = (session, kind, payload) =>
    window.postMessage({ channel: CHANNEL, to: 'page', session, kind, payload }, window.location.origin);

  window.addEventListener('message', (event) => {
    if (event.source !== window || event.origin !== window.location.origin) return;
    const data = event.data;
    if (!data || data.channel !== CHANNEL || data.to !== 'extension' || typeof data.session !== 'string') return;
    const { session, kind } = data;
    if (kind === 'open') {
      if (ports.has(session)) return;
      const port = api.runtime.connect({ name: CHANNEL });
      ports.set(session, port);
      port.onMessage.addListener((payload) => toPage(session, 'message', payload));
      port.onDisconnect.addListener(() => {
        if (ports.delete(session)) toPage(session, 'closed');
      });
      toPage(session, 'opened');
    } else if (kind === 'message') {
      ports.get(session)?.postMessage(data.payload);
    } else if (kind === 'close') {
      const port = ports.get(session);
      ports.delete(session);
      port?.disconnect();
    }
  });
})();
