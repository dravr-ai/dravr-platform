// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins which bundles carry the E2E seeded-session routes: Metro blocks app/(auth)/e2e/ unless the flag, a development build and a local API all hold
// ABOUTME: A blocked file never reaches Expo Router's route context, so a store build has no route for a sign-in link (carnet#1201)

import { execFileSync, type ExecFileSyncOptions } from 'child_process';
import path from 'path';

const projectRoot = path.resolve(__dirname, '..');
const ROUTE = path.join(projectRoot, 'app', '(auth)', 'e2e', 'session.tsx');
// Any other file in the directory: the whole of it is E2E-only, not one name.
const SIBLING = path.join(projectRoot, 'app', '(auth)', 'e2e', 'another-route.tsx');
const NEIGHBOUR = path.join(projectRoot, 'app', '(auth)', 'auth', 'callback.tsx');

/**
 * Load metro.config.js the way Expo's CLI does — in a fresh Node process
 * whose environment the bundler was started with — and report whether its
 * block list drops each file. A child process, because the config reads the
 * environment once at load and Jest's own NODE_ENV is `test`.
 */
type Blocked = { route: boolean; sibling: boolean; neighbour: boolean };

function blocked(env: Record<string, string>): Blocked {
  const script = `
    const config = require('./metro.config.js');
    const list = [].concat(config.resolver.blockList || []);
    const drops = (file) => list.some((pattern) => pattern.test(file));
    process.stdout.write(JSON.stringify({
      route: drops(${JSON.stringify(ROUTE)}),
      sibling: drops(${JSON.stringify(SIBLING)}),
      neighbour: drops(${JSON.stringify(NEIGHBOUR)}),
    }));
  `;
  const inherited = Object.fromEntries(
    Object.entries(process.env).filter(
      ([key]) => !['EXPO_PUBLIC_E2E_SEEDED_SESSIONS', 'NODE_ENV', 'EXPO_PUBLIC_API_URL'].includes(key),
    ),
  );
  const out = execFileSync(process.execPath, ['-e', script], {
    cwd: projectRoot,
    // The project types NODE_ENV as always present; the child is given one only when a case names it.
    env: { ...inherited, ...env } as ExecFileSyncOptions['env'],
    encoding: 'utf8',
  });
  return JSON.parse(out) as Blocked;
}

describe('the bundles that carry the seeded-session route', () => {
  // Each case loads Expo's Metro config in a new process.
  jest.setTimeout(60000);

  const E2E = { NODE_ENV: 'development', EXPO_PUBLIC_E2E_SEEDED_SESSIONS: '1' };
  const ALL_BLOCKED: Blocked = { route: true, sibling: true, neighbour: false };

  it('serves the directory to the iOS lane: flag, development build, loopback API', () => {
    expect(blocked({ ...E2E, EXPO_PUBLIC_API_URL: 'http://127.0.0.1:8081' })).toEqual({
      route: false,
      sibling: false,
      neighbour: false,
    });
  });

  it('serves it to the Android lane, whose API is the emulator alias for the host', () => {
    expect(blocked({ ...E2E, EXPO_PUBLIC_API_URL: 'http://10.0.2.2:8081' }).route).toBe(false);
    expect(blocked({ ...E2E, EXPO_PUBLIC_API_URL: 'http://localhost:8091' }).route).toBe(false);
  });

  it('leaves the whole directory out of a development bundle started without the E2E flag', () => {
    expect(blocked({ NODE_ENV: 'development', EXPO_PUBLIC_API_URL: 'http://127.0.0.1:8081' })).toEqual(ALL_BLOCKED);
  });

  it('leaves it out when the flag is anything but 1', () => {
    expect(
      blocked({ ...E2E, EXPO_PUBLIC_E2E_SEEDED_SESSIONS: 'true', EXPO_PUBLIC_API_URL: 'http://127.0.0.1:8081' }),
    ).toEqual(ALL_BLOCKED);
  });

  it('leaves it out of a release export even with the flag set', () => {
    expect(
      blocked({ ...E2E, NODE_ENV: 'production', EXPO_PUBLIC_API_URL: 'http://127.0.0.1:8081' }),
    ).toEqual(ALL_BLOCKED);
  });

  it('leaves it out unless NODE_ENV says development: unset or test is not enough', () => {
    expect(
      blocked({ EXPO_PUBLIC_E2E_SEEDED_SESSIONS: '1', EXPO_PUBLIC_API_URL: 'http://127.0.0.1:8081' }),
    ).toEqual(ALL_BLOCKED);
    expect(blocked({ ...E2E, NODE_ENV: 'test', EXPO_PUBLIC_API_URL: 'http://127.0.0.1:8081' })).toEqual(ALL_BLOCKED);
  });

  it('leaves it out of a bundle pointed at a real server, or at none', () => {
    expect(blocked({ ...E2E, EXPO_PUBLIC_API_URL: 'https://app.dravr.ai' })).toEqual(ALL_BLOCKED);
    expect(blocked({ ...E2E, EXPO_PUBLIC_API_URL: 'http://192.168.1.20:8081' })).toEqual(ALL_BLOCKED);
    expect(blocked({ ...E2E, EXPO_PUBLIC_API_URL: 'http://127.0.0.1.example.com:8081' })).toEqual(ALL_BLOCKED);
    expect(blocked(E2E)).toEqual(ALL_BLOCKED);
  });
});
