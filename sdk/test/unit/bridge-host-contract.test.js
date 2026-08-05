// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the bridge's contract with its MCP host: declared capabilities and request budget
// ABOUTME: Proves tools.listChanged is declared and every browser wait answers inside the host deadline

const { Client } = require('@modelcontextprotocol/sdk/client/index.js');
const { InMemoryTransport } = require('@modelcontextprotocol/sdk/inMemory.js');
const {
  DEFAULT_REQUEST_TIMEOUT_MSEC,
} = require('@modelcontextprotocol/sdk/shared/protocol.js');
const {
  ToolListChangedNotificationSchema,
} = require('@modelcontextprotocol/sdk/types.js');
const { PierreError, PierreErrorCode, PierreMcpClient } = require('../../dist/index.js');
const {
  closeServer,
  httpRequest,
  startPierreStub,
  startProvider,
  stopProvider,
} = require('./oauth-callback-harness.js');

/** An access token the bridge can decode a user id out of. */
function jwtFor(subject) {
  const payload = Buffer.from(JSON.stringify({ sub: subject })).toString('base64');
  return `header.${payload}.signature`;
}

function makeBridge(serverUrl, overrides = {}) {
  const bridge = new PierreMcpClient({
    mode: 'oauth',
    pierreServerUrl: serverUrl,
    oauthClientId: 'test-client',
    oauthClientSecret: 'test-secret',
    disableBrowser: true,
    ...overrides,
  });
  bridge.log = () => {};
  return bridge;
}

/**
 * Bridge wired for a connect_provider call: an OAuth session that already holds a token,
 * a bound callback listener, and a Pierre client that reports the provider unconnected.
 */
async function connectProviderBridge() {
  const { provider } = await startProvider({ disableBrowser: true });
  provider.savedTokens = {
    access_token: jwtFor('user-1'),
    refresh_token: 'rotating-refresh',
    token_type: 'Bearer',
    expires_in: 3600,
    scope: 'read:fitness write:fitness',
  };

  const bridge = makeBridge('http://localhost:8081');
  await bridge.createMcpServer();
  bridge.oauthProvider = provider;
  bridge.pierreClient = {
    callTool: async () => ({
      structuredContent: { providers: [{ provider: 'strava', connected: false }] },
    }),
  };

  return { bridge, provider, handler: bridge.mcpServer._requestHandlers.get('tools/call') };
}

const connectProviderRequest = {
  method: 'tools/call',
  params: { name: 'connect_provider', arguments: { provider: 'strava' } },
};

describe('declared tool capabilities', () => {
  test('the host is told the tool list changes, because the bridge tells it that it did', async () => {
    const bridge = makeBridge('http://localhost:8081');
    await bridge.createMcpServer();

    const host = new Client({ name: 'test-host', version: '1.0.0' }, { capabilities: {} });
    const [hostTransport, bridgeTransport] = InMemoryTransport.createLinkedPair();

    const listChanged = [];
    host.setNotificationHandler(ToolListChangedNotificationSchema, async (notification) => {
      listChanged.push(notification);
    });

    try {
      await Promise.all([host.connect(hostTransport), bridge.mcpServer.connect(bridgeTransport)]);

      // A host decides whether to re-fetch tools/list from the declared capability, so the
      // declaration has to be there before the notification means anything.
      expect(host.getServerCapabilities().tools).toEqual({ listChanged: true });

      await bridge.mcpServer.notification({
        method: 'notifications/tools/list_changed',
        params: {},
      });

      // Give the linked transport a turn to deliver.
      await new Promise((resolve) => setImmediate(resolve));
      expect(listChanged).toHaveLength(1);
      expect(listChanged[0].method).toBe('notifications/tools/list_changed');
    } finally {
      await host.close();
      await bridge.mcpServer.close();
    }
  });
});

describe('connect_provider stays inside the host request budget', () => {
  test('the wait is shorter than the host deadline and carries the host cancel signal', async () => {
    const { bridge, provider, handler } = await connectProviderBridge();

    const waits = [];
    let waitEntered;
    const entered = new Promise((resolve) => {
      waitEntered = resolve;
    });
    const realWait = provider.waitForProviderOAuth.bind(provider);
    provider.waitForProviderOAuth = (name, ms, signal) => {
      waits.push({ name, ms, signal });
      waitEntered();
      return realWait(name, ms, signal);
    };

    const controller = new AbortController();

    try {
      const pending = handler(connectProviderRequest, { signal: controller.signal });
      await entered;

      expect(waits[0].name).toBe('strava');
      // A host that sets no timeout of its own abandons the call at the SDK default and
      // reports failure, so the answer has to come back before then.
      expect(waits[0].ms).toBeLessThan(DEFAULT_REQUEST_TIMEOUT_MSEC);
      expect(waits[0].ms).toBe(55000);
      expect(waits[0].signal).toBe(controller.signal);

      controller.abort();
      const result = await pending;

      expect(result.isError).toBe(false);
      expect(result.content[0].text).toMatch(/still open in your browser/i);
      // The cancelled wait left nothing pending behind it.
      expect(provider.pendingProviderOAuth.size).toBe(0);
    } finally {
      stopProvider(provider);
      await bridge.mcpServer.close();
    }
  });

  test('a host offering a progressToken gets progress and the full authorization window', async () => {
    const { bridge, provider, handler } = await connectProviderBridge();

    const waits = [];
    let waitEntered;
    const entered = new Promise((resolve) => {
      waitEntered = resolve;
    });
    const realWait = provider.waitForProviderOAuth.bind(provider);
    provider.waitForProviderOAuth = (name, ms, signal) => {
      waits.push({ name, ms, signal });
      waitEntered();
      return realWait(name, ms, signal);
    };

    const notifications = [];
    const controller = new AbortController();
    const extra = {
      signal: controller.signal,
      _meta: { progressToken: 42 },
      sendNotification: async (notification) => {
        notifications.push(notification);
      },
    };

    try {
      const pending = handler(connectProviderRequest, extra);
      await entered;
      // The wait is entered after the first progress report, so the report is already out.
      expect(waits[0].ms).toBe(120000);

      expect(notifications).toHaveLength(1);
      expect(notifications[0].method).toBe('notifications/progress');
      expect(notifications[0].params.progressToken).toBe(42);
      expect(notifications[0].params.total).toBe(120000);
      expect(notifications[0].params.message).toMatch(/waiting for strava authorization/i);

      controller.abort();
      await pending;
    } finally {
      stopProvider(provider);
      await bridge.mcpServer.close();
    }
  });

  test('an authorization the budget did not outlive is reported as pending, not as a failure', async () => {
    const { bridge, provider, handler } = await connectProviderBridge();
    provider.waitForProviderOAuth = async () => {
      throw new PierreError(
        PierreErrorCode.TIMEOUT_ERROR,
        'strava OAuth timed out after 55000ms',
      );
    };

    try {
      const result = await handler(connectProviderRequest, {
        signal: new AbortController().signal,
      });

      expect(result.isError).toBe(false);
      expect(result.content[0].text).toMatch(/not confirmed yet/i);
      expect(result.content[0].text).toMatch(/confirmation message/i);
      expect(result.content[0].text).toMatch(/run connect_provider again/i);
      // The two claims that were wrong before: it did not fail, and it is not connected.
      expect(result.content[0].text).not.toMatch(/timed out/i);
      expect(result.content[0].text).not.toMatch(/connected successfully/i);
    } finally {
      stopProvider(provider);
      await bridge.mcpServer.close();
    }
  });
});

/**
 * Lifts the switches that make the bridge refuse an interactive sign-in - the jest setup
 * sets PIERRE_DISABLE_BROWSER for the whole run - and returns the restore function. These
 * tests are about the wait a real host gets, which only happens on the interactive path.
 * No browser is ever opened: the browser step itself is replaced below.
 */
function allowInteractiveSignIn() {
  const saved = {
    PIERRE_DISABLE_BROWSER: process.env.PIERRE_DISABLE_BROWSER,
    CI: process.env.CI,
    GITHUB_ACTIONS: process.env.GITHUB_ACTIONS,
  };
  for (const name of Object.keys(saved)) {
    delete process.env[name];
  }
  return () => {
    for (const [name, value] of Object.entries(saved)) {
      if (value === undefined) {
        delete process.env[name];
      } else {
        process.env[name] = value;
      }
    }
  };
}

/**
 * Bridge wired for a Pierre sign-in: a Pierre stub that issues tokens, a real OAuth
 * session with a bound callback listener, and the browser step reduced to what it does
 * besides opening a page - arm the listener and wait for the callback. The transport
 * connect that follows a sign-in is replaced by a client reporting one tool, so the test
 * stays on the sign-in wait and off the network.
 */
async function signInBridge({ authorizationTimeoutMs = 2000 } = {}) {
  const pierre = await startPierreStub({
    '/oauth2/token': () => ({
      body: {
        access_token: 'issued-jwt',
        token_type: 'Bearer',
        expires_in: 3600,
        refresh_token: 'issued-refresh',
        scope: 'read:fitness write:fitness',
      },
    }),
  });

  const { provider, port } = await startProvider({ authorizationTimeoutMs }, pierre.url);
  provider.clientInfo = { client_id: 'client-abc', client_secret: 'secret-xyz' };

  let browserSteps = 0;
  let reachedBrowserStep;
  const atBrowserStep = new Promise((resolve) => {
    reachedBrowserStep = resolve;
  });
  provider.redirectToAuthorization = async () => {
    browserSteps += 1;
    await provider.startCallbackServer();
    reachedBrowserStep();
    await provider.completeAuthorization();
  };

  const bridge = makeBridge(pierre.url, { disableBrowser: false });
  await bridge.createMcpServer();
  bridge.oauthProvider = provider;
  bridge.attemptConnection = async () => {
    bridge.pierreClient = {
      listTools: async () => ({ tools: [{ name: 'get_activities' }] }),
    };
  };

  return {
    bridge,
    provider,
    pierre,
    port,
    atBrowserStep,
    browserSteps: () => browserSteps,
    handler: bridge.mcpServer._requestHandlers.get('tools/call'),
  };
}

async function cleanupSignIn({ bridge, provider, pierre }) {
  stopProvider(provider);
  await bridge.mcpServer.close();
  await closeServer(pierre.server);
}

/**
 * Keeps the real budget decision and the real wait, on a clock a test can afford: the
 * production budget is asserted on its own below, and sitting through it here would cost
 * a minute per test.
 */
function shortenWait(bridge, ms) {
  const realWait = bridge.waitForPierreLogin.bind(bridge);
  bridge.waitForPierreLogin = (login, _budgetMs, signal) => realWait(login, ms, signal);
}

const connectToPierreRequest = {
  method: 'tools/call',
  params: { name: 'connect_to_pierre', arguments: {} },
};

describe('connect_to_pierre stays inside the host request budget', () => {
  test('the wait is shorter than the host deadline and carries the host cancel signal', async () => {
    const restoreEnv = allowInteractiveSignIn();
    const wired = await signInBridge();
    const { bridge, provider, atBrowserStep, handler } = wired;

    const waits = [];
    let waitEntered;
    const entered = new Promise((resolve) => {
      waitEntered = resolve;
    });
    const realWait = bridge.waitForPierreLogin.bind(bridge);
    bridge.waitForPierreLogin = (login, ms, signal) => {
      waits.push({ ms, signal });
      waitEntered();
      return realWait(login, ms, signal);
    };

    const controller = new AbortController();

    try {
      const pending = handler(connectToPierreRequest, { signal: controller.signal });
      await entered;

      // A host that sets no timeout of its own abandons the call at the SDK default and
      // tells the user it failed, so the answer has to come back before then.
      expect(waits[0].ms).toBeLessThan(DEFAULT_REQUEST_TIMEOUT_MSEC);
      expect(waits[0].ms).toBe(55000);
      expect(waits[0].signal).toBe(controller.signal);

      // Cancel with the flow parked on the browser step, which is where a host cancels in
      // practice: the user has the page and has not finished with it.
      await atBrowserStep;
      controller.abort();
      const result = await pending;

      expect(result.isError).toBe(false);
      expect(result.content[0].text).toMatch(/still open in your browser/i);

      // What that reply claims: walking away from the wait left the flow itself running,
      // listener bound and code verifier intact, so the open page can still complete.
      expect(provider.callbackServer).toBeDefined();
      expect(provider.authorizationPending).toBeDefined();
      expect(provider.codeVerifierValue).toBeDefined();
    } finally {
      await cleanupSignIn(wired);
      restoreEnv();
    }
  });

  test('a sign-in the budget did not outlive still completes from the browser afterwards', async () => {
    const restoreEnv = allowInteractiveSignIn();
    const wired = await signInBridge({ authorizationTimeoutMs: 10000 });
    const { bridge, provider, port, handler } = wired;
    shortenWait(bridge, 50);

    try {
      const result = await handler(connectToPierreRequest, {
        signal: new AbortController().signal,
      });

      expect(result.isError).toBe(false);
      expect(result.content[0].text).toMatch(/not confirmed yet/i);
      expect(result.content[0].text).toMatch(/run connect_to_pierre again/i);
      // The two claims that would be wrong: it did not fail, and it is not connected.
      expect(result.content[0].text).not.toMatch(/timed out/i);
      expect(result.content[0].text).not.toMatch(/successfully connected/i);

      // The flow the reply says is still open takes the callback and finishes the sign-in
      // the tool call stopped waiting for.
      const signIn = bridge.pierreLoginInFlight;
      expect(signIn).toBeTruthy();

      const callback = await httpRequest(port, {
        method: 'GET',
        path: `/oauth/callback?code=authorization-code-1&state=${provider.stateValue}`,
      });
      expect(callback.status).toBe(200);
      await signIn;

      const tokens = await provider.tokens();
      expect(tokens.access_token).toBe('issued-jwt');
      expect(tokens.refresh_token).toBe('issued-refresh');
      // Which is what makes "run it again" true: the retry finds a session and a toolset.
      expect(bridge.cachedTools.tools.map((tool) => tool.name)).toEqual(['get_activities']);
      expect(bridge.pierreLoginInFlight).toBeNull();
    } finally {
      await cleanupSignIn(wired);
      restoreEnv();
    }
  });

  test('a host offering a progressToken gets progress and the full sign-in window', async () => {
    const restoreEnv = allowInteractiveSignIn();
    const wired = await signInBridge();
    const { bridge, handler } = wired;

    const waits = [];
    let waitEntered;
    const entered = new Promise((resolve) => {
      waitEntered = resolve;
    });
    const realWait = bridge.waitForPierreLogin.bind(bridge);
    bridge.waitForPierreLogin = (login, ms, signal) => {
      waits.push({ ms, signal });
      waitEntered();
      return realWait(login, ms, signal);
    };

    const notifications = [];
    const controller = new AbortController();
    const extra = {
      signal: controller.signal,
      _meta: { progressToken: 7 },
      sendNotification: async (notification) => {
        notifications.push(notification);
      },
    };

    try {
      const pending = handler(connectToPierreRequest, extra);
      await entered;
      // The wait is entered after the first progress report, so the report is already out.
      expect(waits[0].ms).toBe(120000);

      expect(notifications).toHaveLength(1);
      expect(notifications[0].method).toBe('notifications/progress');
      expect(notifications[0].params.progressToken).toBe(7);
      expect(notifications[0].params.total).toBe(120000);
      expect(notifications[0].params.message).toMatch(/pierre sign-in/i);

      controller.abort();
      await pending;
    } finally {
      await cleanupSignIn(wired);
      restoreEnv();
    }
  });

  test('a second call while a sign-in is pending joins it instead of opening a second one', async () => {
    const restoreEnv = allowInteractiveSignIn();
    const wired = await signInBridge({ authorizationTimeoutMs: 10000 });
    const { bridge, provider, browserSteps, handler } = wired;
    shortenWait(bridge, 50);

    try {
      const first = await handler(connectToPierreRequest, {
        signal: new AbortController().signal,
      });
      const verifier = provider.codeVerifierValue;
      const second = await handler(connectToPierreRequest, {
        signal: new AbortController().signal,
      });

      expect(second.content[0].text).toBe(first.content[0].text);
      // Retrying is what the reply asks for, so the retry must not start a second flow:
      // that would replace the code verifier the page already open was challenged
      // against, leaving neither page able to complete.
      expect(browserSteps()).toBe(1);
      expect(provider.codeVerifierValue).toBe(verifier);
    } finally {
      await cleanupSignIn(wired);
      restoreEnv();
    }
  });
});

describe('a tool call that finds no session', () => {
  test('reports the sign-in instead of running the tool it interrupted', async () => {
    const restoreEnv = allowInteractiveSignIn();
    const wired = await signInBridge();
    const { bridge, provider, handler } = wired;
    shortenWait(bridge, 50);

    let toolCalls = 0;
    bridge.pierreClient = {
      callTool: async () => {
        toolCalls += 1;
        return { content: [{ type: 'text', text: 'activities' }] };
      },
    };

    try {
      const result = await handler(
        { method: 'tools/call', params: { name: 'get_activities', arguments: {} } },
        { signal: new AbortController().signal },
      );

      // The tool the host asked for never ran, so the reply says so and names it.
      expect(toolCalls).toBe(0);
      expect(result.content[0].text).toMatch(/not confirmed yet/i);
      expect(result.content[0].text).toMatch(/run get_activities again/i);
      expect(result.content[0].text).not.toMatch(/activities'/);

      // And the sign-in it started is still open, which is what makes retrying work.
      expect(provider.callbackServer).toBeDefined();
      expect(bridge.pierreLoginInFlight).toBeTruthy();
    } finally {
      await cleanupSignIn(wired);
      restoreEnv();
    }
  });

  test('a sign-in that lands inside the budget runs the tool the host asked for', async () => {
    const restoreEnv = allowInteractiveSignIn();
    const wired = await signInBridge({ authorizationTimeoutMs: 10000 });
    const { bridge, provider, port, handler } = wired;

    // A user who signs in straight away: the callback arrives while the tool call is
    // still waiting, so the call has a session and no reason to hand the work back.
    provider.redirectToAuthorization = async () => {
      await provider.startCallbackServer();
      const completing = provider.completeAuthorization();
      await httpRequest(port, {
        method: 'GET',
        path: `/oauth/callback?code=authorization-code-1&state=${provider.stateValue}`,
      });
      await completing;
    };

    const calls = [];
    bridge.attemptConnection = async () => {
      bridge.pierreClient = {
        listTools: async () => ({ tools: [{ name: 'get_activities' }] }),
        callTool: async (call) => {
          calls.push(call);
          return { content: [{ type: 'text', text: '3 activities' }] };
        },
      };
    };

    try {
      const result = await handler(
        { method: 'tools/call', params: { name: 'get_activities', arguments: { limit: 3 } } },
        { signal: new AbortController().signal },
      );

      expect(calls).toEqual([{ name: 'get_activities', arguments: { limit: 3 } }]);
      expect(result.content[0].text).toBe('3 activities');
    } finally {
      await cleanupSignIn(wired);
      restoreEnv();
    }
  });
});
