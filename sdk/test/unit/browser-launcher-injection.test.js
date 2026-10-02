// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Regression tests proving a shell payload in an OAuth URL fragment is never executed
// ABOUTME: Drives the launcher and both of its callers with a hostile URL and inspects every child process started

const { readFileSync, writeFileSync, mkdtempSync, existsSync, rmSync } = require('fs');
const { join } = require('path');
const { tmpdir } = require('os');
const { transformSync } = require('esbuild');
const childProcess = require('child_process');
const { freePort, makeProvider, stopProvider } = require('./oauth-callback-harness.js');
const { fastPoll, startDravr, wiredBridge } = require('../helpers/provider-connect-dravr.js');

// The launcher is internal to the SDK (not re-exported from dist/index.js), so the
// test compiles the real source with the same TypeScript-to-CJS transform the build
// uses and requires the result. The code under test is therefore the shipped code.
// The source is read here to be compiled and run, never to be matched as text.
const LAUNCHER_SRC = join(__dirname, '..', '..', 'src', 'browser-launcher.ts');

// The proven vector: the fragment survives URL serialization byte-for-byte, so a
// command substitution written there reaches whatever the launcher hands the URL to.
const PAYLOAD_DIR = mkdtempSync(join(tmpdir(), 'pierre-launcher-'));
const shellPayloadUrl = (marker) =>
  `https://attacker.example/authorize#$(touch\${IFS}${join(PAYLOAD_DIR, marker)})`;

const realExecFileSync = childProcess.execFileSync;
const realExec = childProcess.exec;
const realExecSync = childProcess.execSync;
const realExecFile = childProcess.execFile;
const realSpawn = childProcess.spawn;
const realSpawnSync = childProcess.spawnSync;
const realPlatform = process.platform;

let launcher;
let launched;
let shellCalls;
let logs;

function setPlatform(platform) {
  Object.defineProperty(process, 'platform', { value: platform, configurable: true });
}

beforeAll(() => {
  const { code } = transformSync(readFileSync(LAUNCHER_SRC, 'utf-8'), {
    loader: 'ts',
    format: 'cjs',
    target: 'node24',
  });
  const compiled = join(PAYLOAD_DIR, 'browser-launcher.js');
  writeFileSync(compiled, code);
  launcher = require(compiled);
});

beforeEach(() => {
  launched = [];
  shellCalls = [];
  logs = [];

  // The macOS path schedules browser activation 500ms after the URL opens. Fake
  // timers keep that under the test's control: tests that assert on activation
  // advance the clock, and the pending timer of every other test is discarded at
  // teardown rather than firing a real osascript once the stubs are restored.
  jest.useFakeTimers();

  // The jest setup file sets PIERRE_DISABLE_BROWSER for the whole run; clear the
  // kill switch so the launch path is actually reached (nothing real is spawned -
  // exec/execFile are captured below).
  delete process.env.PIERRE_DISABLE_BROWSER;
  delete process.env.CI;
  delete process.env.GITHUB_ACTIONS;

  // execFile is the one way in that takes an argv; options are recorded when given,
  // so a `shell: true` slipped into them shows up in the captured call.
  childProcess.execFile = (file, args, optionsOrCallback, maybeCallback) => {
    const callback = typeof optionsOrCallback === 'function' ? optionsOrCallback : maybeCallback;
    const options = typeof optionsOrCallback === 'function' ? undefined : optionsOrCallback;
    launched.push(options === undefined ? { file, args } : { file, args, options });
    if (typeof callback === 'function') {
      callback(null, '', '');
    }
  };
  // Every other way to start a process is a finding: exec and execSync parse a command
  // line, and nothing in an OAuth launch has a reason to spawn.
  childProcess.exec = (command, callback) => {
    shellCalls.push(command);
    if (typeof callback === 'function') {
      callback(null, '', '');
    }
  };
  childProcess.execSync = (command) => {
    shellCalls.push(command);
    return '';
  };
  childProcess.spawn = (file, args, spawnOptions) => {
    shellCalls.push({ spawn: file, args, options: spawnOptions });
    throw new Error(`unexpected spawn of ${file} during an OAuth browser launch`);
  };
  childProcess.spawnSync = (file, args, spawnOptions) => {
    shellCalls.push({ spawnSync: file, args, options: spawnOptions });
    throw new Error(`unexpected spawnSync of ${file} during an OAuth browser launch`);
  };
});

afterEach(() => {
  jest.useRealTimers();
  childProcess.exec = realExec;
  childProcess.execSync = realExecSync;
  childProcess.execFile = realExecFile;
  childProcess.spawn = realSpawn;
  childProcess.spawnSync = realSpawnSync;
  setPlatform(realPlatform);
});

afterAll(() => {
  rmSync(PAYLOAD_DIR, { recursive: true, force: true });
});

const options = () => ({ disableBrowser: false, log: (message) => logs.push(message) });

describe('browser launcher command injection', () => {
  test('the fragment payload survives URL serialization, so it does reach the launcher', () => {
    const url = shellPayloadUrl('serialization');
    expect(new URL(url).href).toBe(url);
    expect(url).toContain('$(touch');
    expect(url).toContain('${IFS}');
  });

  test('the same payload IS executed when a URL is interpolated into a shell string', () => {
    if (realPlatform === 'win32') {
      return; // POSIX shell control case
    }
    const marker = join(PAYLOAD_DIR, 'shell-control');
    const url = `https://attacker.example/authorize#$(touch\${IFS}${marker})`;

    // The shape the vulnerable code used: the URL inside a double-quoted shell word.
    realExecFileSync('/bin/sh', ['-c', `printf '%s' "${url}"`]);

    expect(existsSync(marker)).toBe(true);
  });

  test('macOS hands the URL to open as one argv element and never runs a shell', () => {
    setPlatform('darwin');
    const url = shellPayloadUrl('darwin');

    launcher.openUrlInBrowserWithFocus(url, options());

    expect(shellCalls).toEqual([]);
    expect(launched).toHaveLength(1);
    expect(launched[0].file).toBe('open');
    expect(launched[0].args).toEqual([url]);
  });

  test('argv delivery is inert: running the captured argv creates nothing', () => {
    if (realPlatform === 'win32') {
      return; // /bin/echo is the POSIX stand-in for the platform opener
    }
    setPlatform('darwin');
    const marker = join(PAYLOAD_DIR, 'argv-inert');
    const url = `https://attacker.example/authorize#$(touch\${IFS}${marker})`;

    launcher.openUrlInBrowserWithFocus(url, options());

    // Feed the argv the launcher produced to a real process, exactly as execFile would.
    const printed = realExecFileSync('/bin/echo', launched[0].args).toString();

    expect(printed.trim()).toBe(url); // delivered verbatim, as data
    expect(existsSync(marker)).toBe(false); // and never interpreted
  });

  test('Linux hands the URL to xdg-open as one argv element', () => {
    setPlatform('linux');
    const url = shellPayloadUrl('linux');

    launcher.openUrlInBrowserWithFocus(url, options());

    expect(shellCalls).toEqual([]);
    expect(launched).toEqual([{ file: 'xdg-open', args: [url] }]);
  });

  test('Windows opens through rundll32, not a cmd.exe command line', () => {
    setPlatform('win32');
    const url = 'https://attacker.example/authorize#&calc.exe';

    launcher.openUrlInBrowserWithFocus(url, options());

    expect(shellCalls).toEqual([]);
    expect(launched).toHaveLength(1);
    expect(launched[0].file).toBe('rundll32.exe');
    // cmd.exe re-parses its arguments, where an unquoted `&` starts a second command.
    expect(launched[0].file).not.toMatch(/cmd\.exe/i);
    expect(launched[0].args).toEqual(['url.dll,FileProtocolHandler', url]);
  });

  test('macOS browser activation runs fixed AppleScript with no URL in it', () => {
    setPlatform('darwin');
    const url = shellPayloadUrl('activation');

    launcher.openUrlInBrowserWithFocus(url, options());
    jest.advanceTimersByTime(500);

    // The stub reports success for every candidate, so the chain stops at the first
    // one - and the AppleScript it runs carries no part of the URL.
    const activations = launched.filter((call) => call.file === 'osascript');
    expect(activations).toHaveLength(1);
    expect(activations[0].args).toEqual([
      '-e',
      'tell application "Google Chrome" to activate',
    ]);
    for (const call of launched) {
      if (call.file !== 'open') {
        expect(call.args.join(' ')).not.toContain('$(touch');
      }
    }
  });

  test('the activation chain falls through to the other browsers on failure', () => {
    setPlatform('darwin');
    childProcess.execFile = (file, args, callback) => {
      launched.push({ file, args });
      callback(file === 'osascript' ? new Error('not running') : null, '', '');
    };

    launcher.openUrlInBrowserWithFocus('https://pierre.example/oauth2/login', options());
    jest.advanceTimersByTime(500);

    expect(launched.filter((call) => call.file === 'osascript').map((call) => call.args[1])).toEqual([
      'tell application "Google Chrome" to activate',
      'tell application "Safari" to activate',
      'tell application "Firefox" to activate',
      'tell application "Brave Browser" to activate',
    ]);
    expect(logs).toContain('Could not activate browser (non-fatal)');
  });

  test('non-HTTP schemes are refused before any process starts', () => {
    setPlatform('darwin');

    for (const url of ['javascript:alert(1)', 'file:///etc/passwd', 'data:text/html,x']) {
      launcher.openUrlInBrowserWithFocus(url, options());
    }
    launcher.openUrlInBrowserWithFocus('not a url', options());

    expect(launched).toEqual([]);
    expect(shellCalls).toEqual([]);
    expect(logs).toContain('Refusing to open non-HTTP URL: javascript:');
    expect(logs).toContain('Invalid URL format, refusing to open');
  });

  test('the kill switch suppresses the launch and logs the URL instead', () => {
    setPlatform('darwin');
    const url = shellPayloadUrl('killswitch');

    launcher.openUrlInBrowserWithFocus(url, { disableBrowser: true, log: (m) => logs.push(m) });
    process.env.PIERRE_DISABLE_BROWSER = 'true';
    launcher.openUrlInBrowserWithFocus(url, options());
    process.env.PIERRE_DISABLE_BROWSER = 'false';
    process.env.CI = 'true';
    launcher.openUrlInBrowserWithFocus(url, options());

    expect(launched).toEqual([]);
    expect(shellCalls).toEqual([]);
    expect(logs.filter((m) => m === `OAuth URL: ${url}`)).toHaveLength(3);
  });
});

/**
 * The launcher is safe only for callers that go through it. Both places the SDK
 * opens a browser are driven here, from their public entry point, with the URL an
 * attacker controls: the authorization endpoint a server's discovery document names,
 * and the provider page Dravr's connect_provider tool mints. A caller that built its
 * own command, or stopped using the launcher, would show up as a shell call or as a
 * different argv. The rule that no file under src/ can import a shell API at all is
 * ESLint's (eslint.config.js, proved in shell-free-lint.test.js).
 *
 * Linux is the platform under test because its launch is one execFile with no
 * follow-up: macOS schedules a browser activation that would outlive these tests'
 * real timers. The per-platform argv is covered above.
 */
describe('both callers hand a hostile URL to the launcher, never to a shell', () => {
  beforeEach(() => {
    // These drive real sockets and a real authorization timeout.
    jest.useRealTimers();
    setPlatform('linux');
  });

  test('the OAuth session opens the authorization endpoint as one argv element', async () => {
    const url = shellPayloadUrl('session-caller');
    const provider = makeProvider({
      disableBrowser: false,
      callbackPort: await freePort(),
      authorizationTimeoutMs: 150,
    });

    try {
      // Nobody completes the authorization, so the flow ends on its own timeout; by
      // then the browser launch has happened.
      await expect(provider.redirectToAuthorization(new URL(url))).rejects.toThrow(
        /authorization was not completed within/i,
      );
    } finally {
      stopProvider(provider);
    }

    expect(shellCalls).toEqual([]);
    expect(launched).toEqual([{ file: 'xdg-open', args: [url] }]);
    expect(existsSync(join(PAYLOAD_DIR, 'session-caller'))).toBe(false);
  });

  test('connect_provider opens the page Dravr minted as one argv element', async () => {
    const url = shellPayloadUrl('bridge-caller');
    const dravr = await startDravr(['disconnected', 'connected'], url);
    const wired = await wiredBridge(dravr, undefined, { disableBrowser: false });
    fastPoll(wired.bridge);

    try {
      const result = await wired.connect();
      expect(result.isError).toBe(false);
    } finally {
      await wired.cleanup();
      await dravr.close();
    }

    expect(dravr.mints()).toHaveLength(1);
    expect(shellCalls).toEqual([]);
    expect(launched).toEqual([{ file: 'xdg-open', args: [url] }]);
    expect(existsSync(join(PAYLOAD_DIR, 'bridge-caller'))).toBe(false);
  });
});
