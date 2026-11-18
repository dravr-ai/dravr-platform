// ABOUTME: Tests for bridge.js, the content script that relays app.dravr.ai's window messages to a port into the extension
// ABOUTME: Runs the classic script's source against a fake window and runtime under `bun test extensions/dravr-signin` (carnet#1617)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { beforeEach, describe, expect, it } from 'bun:test';

const SOURCE = readFileSync(join(import.meta.dir, 'bridge.js'), 'utf8');
const ORIGIN = 'https://app.dravr.ai';

/** A minimal event: listeners, and a way for the test to fire it. */
function event() {
  const listeners = [];
  return { addListener: (l) => listeners.push(l), fire: (...args) => listeners.forEach((l) => l(...args)) };
}

let win;
let posted;
let ports;

/** Run bridge.js as a browser loads a content script: with `window` and `chrome` as globals. */
function load({ withBrowser = false } = {}) {
  posted = [];
  ports = [];
  const listeners = [];
  win = {
    location: { origin: ORIGIN },
    addEventListener: (type, l) => type === 'message' && listeners.push(l),
    postMessage: (data, targetOrigin) => posted.push({ data, targetOrigin }),
    dispatch: (data, { origin = ORIGIN, source } = {}) =>
      listeners.forEach((l) => l({ data, origin, source: source === undefined ? win : source })),
  };
  const runtime = {
    connect: ({ name }) => {
      const port = { name, sent: [], disconnected: false, onMessage: event(), onDisconnect: event() };
      port.postMessage = (message) => port.sent.push(message);
      port.disconnect = () => {
        port.disconnected = true;
      };
      ports.push(port);
      return port;
    },
  };
  const api = { runtime };
  const run = new Function('window', 'chrome', 'browser', 'globalThis', SOURCE);
  run(win, withBrowser ? undefined : api, withBrowser ? api : undefined, { chrome: withBrowser ? undefined : api, browser: withBrowser ? api : undefined });
}

const toExtension = (session, kind, payload) => ({ channel: 'dravr-signin', to: 'extension', session, kind, payload });

describe('bridge.js', () => {
  beforeEach(() => load());

  it('opens a port for a session and confirms it to the page', () => {
    win.dispatch(toExtension('s1', 'open'));
    expect(ports.map((port) => port.name)).toEqual(['dravr-signin']);
    expect(posted).toEqual([
      { data: { channel: 'dravr-signin', to: 'page', session: 's1', kind: 'opened', payload: undefined }, targetOrigin: ORIGIN },
    ]);
  });

  it('carries the page message onto the port, and the port answer back to the page', () => {
    win.dispatch(toExtension('s1', 'open'));
    win.dispatch(toExtension('s1', 'message', { type: 'ping' }));
    expect(ports[0].sent).toEqual([{ type: 'ping' }]);
    ports[0].onMessage.fire({ type: 'pong', version: '0.1.0' });
    expect(posted.at(-1).data).toEqual({
      channel: 'dravr-signin',
      to: 'page',
      session: 's1',
      kind: 'message',
      payload: { type: 'pong', version: '0.1.0' },
    });
  });

  it('closes the port when the page closes the session, and tells the page when the extension does', () => {
    win.dispatch(toExtension('s1', 'open'));
    win.dispatch(toExtension('s1', 'close'));
    expect(ports[0].disconnected).toBe(true);

    win.dispatch(toExtension('s2', 'open'));
    ports[1].onDisconnect.fire();
    expect(posted.at(-1).data.kind).toBe('closed');
    expect(posted.at(-1).data.session).toBe('s2');
  });

  it('relays nothing from another frame, another origin or another channel', () => {
    win.dispatch(toExtension('s1', 'open'), { source: {} });
    win.dispatch(toExtension('s1', 'open'), { origin: 'https://evil.test' });
    win.dispatch({ ...toExtension('s1', 'open'), channel: 'other' });
    win.dispatch({ ...toExtension('s1', 'open'), to: 'page' });
    expect(ports).toEqual([]);
    expect(posted).toEqual([]);
  });

  it("uses Firefox's browser namespace when it is there", () => {
    load({ withBrowser: true });
    win.dispatch(toExtension('s1', 'open'));
    expect(ports.length).toBe(1);
  });
});
