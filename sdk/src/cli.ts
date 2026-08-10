#!/usr/bin/env node
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai


// ABOUTME: Command-line interface for Dravr MCP Client
// ABOUTME: Parses arguments, configures MCP client, and manages process lifecycle
//
// Licensed under either of Apache License, Version 2.0 or MIT License at your option.
// Copyright (c) 2026 dravr.ai

/**
 * Dravr MCP Client CLI
 *
 * MCP-compliant client connecting MCP hosts to Dravr Fitness MCP Server (HTTP + OAuth 2.0)
 */

import { Command } from 'commander';
import { PierreMcpClient, BridgeConfig } from './bridge';
import { version as packageVersion } from '../package.json';

/**
 * Startup diagnostics for troubleshooting a launch.
 *
 * Written to stderr because stdout carries the MCP protocol stream, and printed only
 * under --verbose so that a normal MCP host launch leaves the host log clean.
 * Credential values are never printed, only whether they are present. The resolved auth
 * mode is printed because which credential the environment supplied decides it.
 */
const logStartupDiagnostics = (config: BridgeConfig, callbackPort: string): void => {
  console.error(`pierre-mcp-client ${packageVersion} starting`);
  console.error(`  server URL = ${config.pierreServerUrl}`);
  console.error(`  callback port = ${callbackPort}`);
  console.error(`  auth mode = ${config.mode}`);
  if (config.mode === 'oauth') {
    console.error(`  OAuth client = ${config.oauthClientId || '[dynamic registration]'}`);
  }
  console.error(`  PIERRE_SERVER_URL = ${process.env.PIERRE_SERVER_URL || '[NOT SET]'}`);
  console.error(`  PIERRE_JWT_TOKEN = ${process.env.PIERRE_JWT_TOKEN ? '[SET]' : '[NOT SET]'}`);
  console.error(`  NODE_ENV = ${process.env.NODE_ENV || '[NOT SET]'}`);
  console.error(`  CI = ${process.env.CI || '[NOT SET]'}`);
};

const program = new Command();

program
  .name('pierre-mcp-client')
  .description('MCP client connecting to Dravr Fitness MCP Server')
  .version(packageVersion)
  .option('-s, --server <url>', 'Dravr MCP server URL', process.env.PIERRE_SERVER_URL || 'http://localhost:8081')
  .option('--oauth-client-id <id>', 'OAuth 2.0 client ID', process.env.PIERRE_OAUTH_CLIENT_ID)
  .option('--callback-port <port>', 'OAuth callback server port', process.env.PIERRE_CALLBACK_PORT || '35535')
  .option('--no-browser', 'Disable automatic browser opening for OAuth (testing mode)')
  .option('--token-validation-timeout <ms>', 'Token validation timeout in milliseconds (default: 3000)', process.env.PIERRE_TOKEN_VALIDATION_TIMEOUT_MS || '3000')
  .option('--proactive-connection-timeout <ms>', 'Proactive connection timeout in milliseconds (default: 5000)', process.env.PIERRE_PROACTIVE_CONNECTION_TIMEOUT_MS || '5000')
  .option('--proactive-tools-list-timeout <ms>', 'Proactive tools list timeout in milliseconds (default: 3000)', process.env.PIERRE_PROACTIVE_TOOLS_LIST_TIMEOUT_MS || '3000')
  .option('--tool-call-connection-timeout <ms>', 'Tool-triggered connection timeout in milliseconds (default: 10000)', process.env.PIERRE_TOOL_CALL_CONNECTION_TIMEOUT_MS || '10000')
  .option('--verbose', 'Print startup diagnostics to stderr')
  .addHelpText(
    'after',
    `
Credentials (environment only, never passed as arguments):
  PIERRE_JWT_TOKEN              Pre-authenticated JWT; selects JWT auth mode
  PIERRE_OAUTH_CLIENT_SECRET    OAuth 2.0 client secret; used with --oauth-client-id

Without either, the client registers an OAuth client dynamically and authorizes in a browser.`,
  )
  .action(async (options) => {
    try {
      // Shared configuration across all auth modes
      const baseConfig = {
        pierreServerUrl: options.server,
        callbackPort: parseInt(options.callbackPort, 10),
        disableBrowser: !options.browser,
        tokenValidationTimeoutMs: parseInt(options.tokenValidationTimeout, 10),
        proactiveConnectionTimeoutMs: parseInt(options.proactiveConnectionTimeout, 10),
        proactiveToolsListTimeoutMs: parseInt(options.proactiveToolsListTimeout, 10),
        toolCallConnectionTimeoutMs: parseInt(options.toolCallConnectionTimeout, 10)
      };

      // Credentials come from the environment and never from argv: process arguments are
      // world-readable for the life of the process (`ps -ef`, /proc/<pid>/cmdline) and they
      // also persist in shell history and in the MCP host's configuration file.
      const jwtToken = process.env.PIERRE_JWT_TOKEN;
      const oauthClientSecret = process.env.PIERRE_OAUTH_CLIENT_SECRET;

      // Determine auth mode based on the credentials available (priority order)
      let config: BridgeConfig;
      if (jwtToken) {
        config = { ...baseConfig, mode: 'jwt', jwtToken };
      } else if (options.oauthClientId && oauthClientSecret) {
        config = { ...baseConfig, mode: 'oauth', oauthClientId: options.oauthClientId, oauthClientSecret };
      } else {
        // Default to OAuth mode without pre-configured client (will use dynamic registration)
        config = { ...baseConfig, mode: 'oauth', oauthClientId: '', oauthClientSecret: '' };
      }

      if (options.verbose) {
        logStartupDiagnostics(config, options.callbackPort);
      }

      const bridge = new PierreMcpClient(config);

      await bridge.start();

      // Store bridge instance for cleanup on shutdown
      (global as any).__bridge = bridge;
    } catch (error) {
      console.error('Bridge failed to start:', error);
      process.exit(1);
    }
  });

// Handle graceful shutdown
let shutdownInProgress = false;

const handleShutdown = (signal: string) => {
  if (shutdownInProgress) {
    console.error('\n⚠️  Forcing immediate exit...');
    process.exit(1);
  }

  shutdownInProgress = true;
  console.error(`\n🛑 Bridge shutting down (${signal})...`);

  const bridge = (global as any).__bridge;
  if (bridge) {
    bridge.stop()
      .then(() => {
        console.error('✅ Bridge stopped cleanly');
        process.exit(0);
      })
      .catch((error: any) => {
        console.error('Error during shutdown:', error);
        process.exit(1);
      });
  } else {
    process.exit(0);
  }
};

process.on('SIGINT', () => handleShutdown('SIGINT'));
process.on('SIGTERM', () => handleShutdown('SIGTERM'));

program.parse();