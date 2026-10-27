// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Runs the pierre-cli built beside the server under test against that server's SQLite database
// ABOUTME: Shared by the control process's seeders (coach plan-thread pairs) and its seeded mobile sessions

import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);

/** How long one `pierre-cli` run may take before the caller fails naming it. */
const CLI_TIMEOUT_MS = 60_000;

/** The CLI built from the same tree as the server, and the database the server serves. */
export interface CliServer {
  /** The `pierre-cli` binary built from the same tree as the server. */
  cliPath: string | undefined;
  /** The SQLite file the server under test uses. */
  databasePath: string | undefined;
}

/**
 * Run `pierre-cli` against the server's database and return what it printed
 * on stdout. The CLI's own error output names what failed; the arguments are
 * never echoed, since they can carry a password, and neither is stdout,
 * since it can carry a token.
 */
export async function runPierreCli(server: CliServer, args: string[]): Promise<string> {
  if (server.cliPath === undefined || server.cliPath === '') {
    throw new Error('PIERRE_CLI names the pierre-cli binary that seeds the server under test');
  }
  if (server.databasePath === undefined || server.databasePath === '') {
    throw new Error('E2E_REAL_DATABASE_PATH names the SQLite file the server under test uses');
  }
  try {
    const { stdout } = await execFileAsync(server.cliPath, args, {
      env: { ...process.env, DATABASE_URL: `sqlite:${server.databasePath}`, RUST_LOG: 'warn' },
      timeout: CLI_TIMEOUT_MS,
    });
    return stdout;
  } catch (error) {
    const failed = error as { stderr?: string; code?: number | string };
    const why = failed.stderr?.trim() || `exit ${String(failed.code)}`;
    throw new Error(`pierre-cli ${args.slice(0, 2).join(' ')} failed: ${why}`, { cause: error });
  }
}
