// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Mock MCP client for testing bridge stdio communication
// ABOUTME: Simulates Claude Desktop's MCP Client behavior over stdin/stdout
//
// Licensed under either of Apache License, Version 2.0 or MIT License at your option.
// Copyright (c) 2026 dravr.ai

const { spawn } = require('child_process');
const { EventEmitter } = require('events');

/**
 * Mock MCP client that communicates via stdio
 * Simulates MCP Client behavior
 */
class MockMCPClient extends EventEmitter {
  /**
   * @param {string} command - executable to spawn (the bridge is `node dist/cli.js`)
   * @param {string[]} args - arguments for that executable
   * @param {{env?: Record<string,string>}} options - `env` entries are merged over the
   *   inherited environment. Credentials travel here, never in `args`: process arguments
   *   are world-readable (`ps -ef`, /proc/<pid>/cmdline), so the client takes its JWT from
   *   PIERRE_JWT_TOKEN and its OAuth secret from PIERRE_OAUTH_CLIENT_SECRET.
   */
  constructor(command, args = [], options = {}) {
    super();
    this.command = command;
    // ALWAYS add --no-browser to prevent 100+ Chrome tabs opening during tests
    this.args = args.includes('--no-browser') ? args : [...args, '--no-browser'];
    this.env = { ...process.env, ...options.env };
    this.process = null;
    this.buffer = '';
    this.pendingRequests = new Map();
    this.nextId = 1;
  }

  async start() {
    return new Promise((resolve, reject) => {
      this.process = spawn(this.command, this.args, {
        stdio: ['pipe', 'pipe', 'pipe'],
        env: this.env
      });

      this.process.on('error', (error) => {
        reject(new Error(`Failed to start bridge: ${error.message}`));
      });

      this.process.stdout.on('data', (data) => {
        this.handleData(data.toString());
      });

      this.process.stderr.on('data', (data) => {
        if (process.env.DEBUG) {
          console.error(`[Bridge Stderr]: ${data}`);
        }
      });

      this.process.on('exit', (code) => {
        if (code !== 0 && code !== null) {
          this.emit('error', new Error(`Bridge exited with code ${code}`));
        }
      });

      // Wait a moment for bridge to initialize
      setTimeout(resolve, 1000);
    });
  }

  handleData(data) {
    this.buffer += data;

    const lines = this.buffer.split('\n');
    this.buffer = lines.pop() || '';

    for (const line of lines) {
      if (line.trim()) {
        try {
          const message = JSON.parse(line);
          this.handleMessage(message);
        } catch (error) {
          console.error('Failed to parse JSON:', line);
        }
      }
    }
  }

  handleMessage(message) {
    if (message.id !== undefined && this.pendingRequests.has(message.id)) {
      const { resolve, reject, timeout } = this.pendingRequests.get(message.id);
      clearTimeout(timeout);
      this.pendingRequests.delete(message.id);

      if (message.error) {
        reject(new Error(message.error.message || JSON.stringify(message.error)));
      } else {
        resolve(message);
      }
    } else {
      this.emit('notification', message);
    }
  }

  async send(request, timeoutMs = 30000) {
    if (!this.process) {
      throw new Error('Client not started');
    }

    const id = request.id || this.nextId++;
    const fullRequest = { ...request, id, jsonrpc: '2.0' };

    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pendingRequests.delete(id);
        reject(new Error(`Request ${id} timed out after ${timeoutMs}ms`));
      }, timeoutMs);

      this.pendingRequests.set(id, { resolve, reject, timeout });

      this.process.stdin.write(JSON.stringify(fullRequest) + '\n');
    });
  }

  sendRaw(data) {
    return new Promise((resolve) => {
      let response = '';
      const originalHandler = this.handleData.bind(this);

      this.handleData = (data) => {
        response += data;
        if (response.includes('\n')) {
          this.handleData = originalHandler;
          resolve(response.trim());
        }
      };

      this.process.stdin.write(data);
    });
  }

  async stop() {
    if (!this.process) return;
    const proc = this.process;
    // Keep `this.process` referencing the child (do NOT null it) so callers can
    // still inspect `.killed` / `.exitCode` after stop() — integration tests assert
    // termination that way.
    await new Promise((resolve) => {
      let settled = false;
      const finish = () => {
        if (!settled) {
          settled = true;
          resolve();
        }
      };
      proc.on('exit', finish);
      try {
        proc.kill('SIGTERM');
      } catch {
        // already exited
        finish();
        return;
      }
      // Unconditional SIGKILL fallback. `proc.killed` only means "a signal was
      // delivered", NOT "the process exited" — gating SIGKILL on it lets a bridge
      // that ignores SIGTERM (e.g. blocked in async work) survive as an orphan.
      setTimeout(() => {
        try {
          proc.kill('SIGKILL');
        } catch {
          // already exited
        }
        finish();
      }, 3000);
    });
  }
}

module.exports = { MockMCPClient };
