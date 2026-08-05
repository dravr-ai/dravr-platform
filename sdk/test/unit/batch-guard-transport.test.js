// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Regression tests proving the batch guard answers batches without stranding messages
// ABOUTME: Feeds real stdin chunks through a real StdioServerTransport and asserts what is delivered

const { buildSync } = require('esbuild');
const { mkdtempSync, rmSync } = require('fs');
const { join } = require('path');
const { tmpdir } = require('os');
const { PassThrough } = require('stream');

// The transport under test is the SDK's own StdioServerTransport, so the drain loop that
// decides how many messages leave one stdin chunk is the shipped implementation, not a stand-in.
const { StdioServerTransport } = require('@modelcontextprotocol/sdk/server/stdio.js');

const SRC = join(__dirname, '..', '..', 'src', 'batch-guard-transport.ts');
const BUILD_DIR = mkdtempSync(join(tmpdir(), 'pierre-batch-guard-'));

let installBatchGuard;
let createBatchGuardMessageHandler;

beforeAll(() => {
  // The guard is internal to the SDK (not re-exported from dist/index.js), so it is bundled
  // with the same esbuild settings the package build uses and required from the output.
  const compiled = join(BUILD_DIR, 'batch-guard-transport.js');
  buildSync({
    entryPoints: [SRC],
    outfile: compiled,
    bundle: true,
    platform: 'node',
    target: 'node24',
    format: 'cjs',
    mainFields: ['module', 'main'],
    logLevel: 'silent',
  });
  ({ installBatchGuard, createBatchGuardMessageHandler } = require(compiled));
});

afterAll(() => {
  rmSync(BUILD_DIR, { recursive: true, force: true });
});

const line = (message) => `${JSON.stringify(message)}\n`;

const request = (id, toolName, limit) => ({
  jsonrpc: '2.0',
  id,
  method: 'tools/call',
  params: { name: toolName, arguments: { limit } },
});

const BATCH = [
  { jsonrpc: '2.0', id: 'batch-1', method: 'tools/list', params: {} },
  { jsonrpc: '2.0', id: 'batch-2', method: 'tools/list', params: {} },
];

let harness;

// stdin data events and transport.send() both resolve on later event-loop turns, so a
// handful of macrotask turns separate "written the chunk" from "observed everything it caused".
const flush = async (turns = 5) => {
  for (let i = 0; i < turns; i += 1) {
    await new Promise((resolve) => setImmediate(resolve));
  }
};

const newHarness = async () => {
  const stdin = new PassThrough();
  const stdout = new PassThrough();
  const transport = new StdioServerTransport(stdin, stdout);

  const delivered = [];
  const errors = [];
  const written = [];
  const logs = [];
  const log = (message) => logs.push(message);

  stdout.on('data', (chunk) => written.push(chunk.toString('utf8')));

  // Same install order as the bridge: guard the transport, start it, then wrap the
  // handler the MCP Server installed so the wrapper sits closest to the transport.
  installBatchGuard(transport, log);
  await transport.start();
  transport.onerror = (error) => errors.push(error);
  transport.onmessage = createBatchGuardMessageHandler(
    transport,
    (message) => delivered.push(message),
    log,
  );

  const feed = async (chunk) => {
    stdin.write(Buffer.from(chunk, 'utf8'));
    await flush();
  };

  const sent = () =>
    written
      .join('')
      .split('\n')
      .filter((entry) => entry.length > 0)
      .map((entry) => JSON.parse(entry));

  return { transport, delivered, errors, logs, feed, sent };
};

beforeEach(async () => {
  harness = await newHarness();
});

afterEach(async () => {
  await harness.transport.close();
});

const expectBatchRejection = (responses) => {
  expect(responses).toHaveLength(1);
  expect(responses[0]).toEqual([
    {
      jsonrpc: '2.0',
      id: 'batch-1',
      error: {
        code: -32600,
        message: 'Batch requests are not supported in protocol version 2025-06-18',
      },
    },
    {
      jsonrpc: '2.0',
      id: 'batch-2',
      error: {
        code: -32600,
        message: 'Batch requests are not supported in protocol version 2025-06-18',
      },
    },
  ]);
};

describe('Batch guard delivery', () => {
  test('delivers a request that shares one chunk with a preceding batch', async () => {
    await harness.feed(line(BATCH) + line(request('after-batch', 'get_activities', 7)));

    expect(harness.delivered).toHaveLength(1);
    expect(harness.delivered[0].id).toBe('after-batch');
    expect(harness.delivered[0].method).toBe('tools/call');
    expect(harness.delivered[0].params.name).toBe('get_activities');
    expect(harness.delivered[0].params.arguments.limit).toBe(7);
    expect(harness.errors).toHaveLength(0);
    expectBatchRejection(harness.sent());
  });

  test('answers a batch that follows a request in the same chunk', async () => {
    await harness.feed(line(request('before-batch', 'get_athlete', 1)) + line(BATCH));

    expect(harness.delivered).toHaveLength(1);
    expect(harness.delivered[0].id).toBe('before-batch');
    expect(harness.delivered[0].params.name).toBe('get_athlete');
    expect(harness.errors).toHaveLength(0);
    expectBatchRejection(harness.sent());
  });

  test('delivers every message queued behind a batch in one chunk', async () => {
    await harness.feed(
      line(BATCH) +
        line(request('q1', 'get_activities', 1)) +
        line(request('q2', 'get_athlete', 2)) +
        line(request('q3', 'get_stats', 3)),
    );

    expect(harness.delivered.map((message) => message.id)).toEqual(['q1', 'q2', 'q3']);
    expect(harness.delivered.map((message) => message.params.name)).toEqual([
      'get_activities',
      'get_athlete',
      'get_stats',
    ]);
    expect(harness.delivered.map((message) => message.params.arguments.limit)).toEqual([1, 2, 3]);
    expectBatchRejection(harness.sent());
  });

  test('holds a partial line until its newline arrives, then delivers it whole', async () => {
    const complete = line(request('split', 'get_activities', 42));
    const cut = complete.indexOf('tools/call') + 4;

    await harness.feed(complete.slice(0, cut));
    expect(harness.delivered).toHaveLength(0);

    await harness.feed(complete.slice(cut));
    expect(harness.delivered).toHaveLength(1);
    expect(harness.delivered[0].id).toBe('split');
    expect(harness.delivered[0].method).toBe('tools/call');
    expect(harness.delivered[0].params.arguments.limit).toBe(42);
  });

  test('keeps draining the chunk after a line that is not JSON', async () => {
    await harness.feed('{ this is not json }\n' + line(request('after-garbage', 'get_stats', 5)));

    expect(harness.errors).toHaveLength(1);
    expect(harness.delivered).toHaveLength(1);
    expect(harness.delivered[0].id).toBe('after-garbage');
    expect(harness.delivered[0].params.arguments.limit).toBe(5);
  });

  test('reports a batch to the message handler rather than the error handler', async () => {
    await harness.feed(line(BATCH));

    expect(harness.errors).toHaveLength(0);
    expect(harness.delivered).toHaveLength(0);
    expectBatchRejection(harness.sent());
  });
});
