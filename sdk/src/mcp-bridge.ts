// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: MCP bridge connecting MCP host (stdio) to Dravr Server (HTTP)
// ABOUTME: Manages MCP message translation, tool forwarding, and OAuth flow integration

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { OAuthClientInformationFull } from "@modelcontextprotocol/sdk/shared/auth.js";
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
  ListResourcesRequestSchema,
  ReadResourceRequestSchema,
  ListPromptsRequestSchema,
  GetPromptRequestSchema,
  CompleteRequestSchema,
  PingRequestSchema,
  SetLevelRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";
import {
  validateMcpToolResponse,
  configureValidator,
  type ResponseValidatorConfig,
} from "./response-validator.js";
import { PierreOAuthClientProvider, OAuthSessionConfig } from "./oauth-session-manager.js";
import { openUrlInBrowserWithFocus } from "./browser-launcher.js";
import { installBatchGuard, createBatchGuardMessageHandler } from "./batch-guard-transport.js";
import { PierreError, PierreErrorCode } from "./errors.js";
import {
  McpHttpClient,
  McpHttpError,
  McpRpcError,
  MCP_PROTOCOL_VERSION,
  RPC_UNSUPPORTED_PROTOCOL_VERSION,
  type McpCallToolResult,
  type McpProgress,
  type McpTask,
  type McpTaskState,
} from "./mcp-http-client.js";
import { version as packageVersion } from "../package.json";

/**
 * How long a tool call waits for a browser step only a human can finish - a Dravr
 * sign-in or a provider authorization - when the host gave it nothing to report progress
 * against.
 *
 * The MCP SDK's default per-request timeout is 60s (DEFAULT_REQUEST_TIMEOUT_MSEC), so a
 * host that sets no timeout of its own abandons the call at that point, emits
 * notifications/cancelled and tells the user the connection failed. The wait therefore
 * ends inside that window and reports what it actually knows, rather than returning a
 * success nobody is listening for any more.
 */
const INTERACTIVE_WAIT_MS = 55000;

/**
 * How long that wait runs when the host sent a progressToken.
 *
 * Progress notifications hold off the host's deadline only for a host that asked for
 * resetTimeoutOnProgress, which defaults to false and is not visible from the server
 * side. So the full human-sized window is spent only when there is a token to keep
 * reporting against; a host that does not extend its deadline cancels instead, and the
 * wait honours the cancellation.
 */
const INTERACTIVE_PROGRESS_WAIT_MS = 120000;

/** Progress cadence during an interactive wait, well inside a 60s deadline. */
const INTERACTIVE_PROGRESS_INTERVAL_MS = 10000;

/** Base configuration shared by all authentication modes */
export interface BridgeConfigBase {
  pierreServerUrl: string;
  callbackPort?: number;
  disableBrowser?: boolean; // Disable browser auto-opening for OAuth (testing mode)
  tokenValidationTimeoutMs?: number; // Default: 3000ms
  proactiveConnectionTimeoutMs?: number; // Default: 5000ms
  proactiveToolsListTimeoutMs?: number; // Default: 3000ms
  toolCallConnectionTimeoutMs?: number; // Default: 10000ms (10s for tool-triggered connections)
  /** Response validation configuration - validates tool responses against Zod schemas */
  responseValidation?: Partial<ResponseValidatorConfig>;
}

/** JWT token authentication mode */
export interface BridgeConfigJwt extends BridgeConfigBase {
  mode: 'jwt';
  jwtToken: string;
}

/** OAuth 2.0 client credentials authentication mode */
export interface BridgeConfigOAuth extends BridgeConfigBase {
  mode: 'oauth';
  oauthClientId: string;
  oauthClientSecret: string;
}

/** API key authentication mode */
export interface BridgeConfigApiKey extends BridgeConfigBase {
  mode: 'api-key';
  apiKey: string;
}

/** Discriminated union for authentication modes */
export type BridgeConfig = BridgeConfigJwt | BridgeConfigOAuth | BridgeConfigApiKey;

export class PierreMcpClient {
  private config: BridgeConfig;
  private pierreClient: McpHttpClient | null = null;
  private mcpServer: Server | null = null;
  private serverTransport: StdioServerTransport | null = null;
  private cachedTools: any = null;
  private proactiveConnectionPromise: Promise<void> | null = null;
  private pierreLoginInFlight: Promise<void> | null = null;
  private oauthProvider: PierreOAuthClientProvider | null = null;
  private mcpUrl: string = "";

  constructor(config: BridgeConfig) {
    this.config = config;

    // Configure response validation if specified
    if (config.responseValidation) {
      configureValidator(config.responseValidation);
    }
  }

  private log(message: string, ...args: any[]) {
    const timestamp = new Date().toISOString();
    console.error(`[${timestamp}] [Dravr Bridge] ${message}`, ...args);
  }

  private async withTimeout<T>(
    promise: Promise<T>,
    timeoutMs: number,
    operation: string,
  ): Promise<T | null> {
    return Promise.race([
      promise,
      new Promise<null>((resolve) =>
        setTimeout(() => {
          this.log(`Operation '${operation}' timed out after ${timeoutMs}ms`);
          resolve(null);
        }, timeoutMs),
      ),
    ]);
  }

  async start(): Promise<void> {
    try {
      // Step 1: Create MCP server for MCP host using stdio
      // This must happen FIRST so the bridge can respond to MCP validator
      await this.createMcpServer();

      // Step 2: Start the bridge (stdio transport)
      await this.startBridge();

      // Step 3: Create MCP client connection to Dravr using Streamable HTTP
      // Initialize in background so MCP server can respond immediately (critical for CI/validators)
      // Connection will complete asynchronously; tools will be available once connected
      // Store promise so tools/list can wait for completion
      this.proactiveConnectionPromise = this.initializePierreConnection()
        .catch((error) => {
          this.log(
            "Dravr connection initialization failed (will retry on first tool call):",
            error,
          );
        })
        .then(() => {
          // Mark promise as resolved
          this.log("Proactive connection promise resolved");
        });

      this.log(
        "Bridge started successfully (Dravr connection initializing in background)",
      );
    } catch (error) {
      this.log("Failed to start bridge:", error);
      throw error;
    }
  }

  private async initializePierreConnection(): Promise<void> {
    // Set up Dravr connection parameters
    this.mcpUrl = `${this.config.pierreServerUrl}/mcp`;

    // Create OAuth provider with callback to notify MCP host when provider OAuth completes
    const onProviderOAuthComplete = async (provider: string): Promise<void> => {
      if (this.mcpServer) {
        const capitalizedProvider =
          provider.charAt(0).toUpperCase() + provider.slice(1);
        await this.mcpServer.notification({
          method: "notifications/message",
          params: {
            level: "info",
            logger: "pierre-oauth",
            data: {
              provider: provider,
              event: "oauth_completed",
              message: `${capitalizedProvider} connected successfully! You can now access your fitness data.`,
            },
          },
        });
        this.log(`Sent ${provider} OAuth completion notification to MCP host`);
      }
    };

    // Convert BridgeConfig to OAuthSessionConfig (both use same discriminated union pattern)
    const baseConfig = {
      pierreServerUrl: this.config.pierreServerUrl,
      callbackPort: this.config.callbackPort,
      disableBrowser: this.config.disableBrowser,
      tokenValidationTimeoutMs: this.config.tokenValidationTimeoutMs,
    };

    let oauthConfig: OAuthSessionConfig;
    switch (this.config.mode) {
      case 'jwt':
        oauthConfig = { ...baseConfig, mode: 'jwt', jwtToken: this.config.jwtToken };
        break;
      case 'oauth':
        oauthConfig = { ...baseConfig, mode: 'oauth', oauthClientId: this.config.oauthClientId, oauthClientSecret: this.config.oauthClientSecret };
        break;
      case 'api-key':
        oauthConfig = { ...baseConfig, mode: 'api-key', apiKey: this.config.apiKey };
        break;
    }

    this.oauthProvider = new PierreOAuthClientProvider(
      this.config.pierreServerUrl,
      oauthConfig,
      onProviderOAuthComplete,
    );

    // Initialize secure storage before any operations that might need it
    await this.oauthProvider.initializeSecureStorage();
    this.log(`Dravr MCP URL configured: ${this.mcpUrl}`);

    // Validate cached tokens and client registration at startup
    // This prevents wasting user time with invalid credentials
    await this.oauthProvider.validateAndCleanupCachedCredentials();

    // Discover proactively and cache the toolset for the MCP host. With a stored
    // session this gives tools/list the real catalogue instead of the connect-only
    // fallback it returns when the server cannot be reached in time; without one the
    // server's answer is its authentication challenge, and the fallback stands until
    // the user signs in. It runs in the background (start() does not await it), so the
    // budget below caps a background task rather than delaying startup. When
    // authentication later swaps in the full toolset, the host is told via
    // notifications/tools/list_changed.
    const connectionTimeoutMs =
      this.config.proactiveConnectionTimeoutMs || 15000;
    const toolsListTimeoutMs = this.config.proactiveToolsListTimeoutMs || 10000;

    try {
      this.log(
        `Connecting to Dravr proactively to cache all tools for MCP host (timeout: ${connectionTimeoutMs}ms)`,
      );
      const connectionResult = await this.withTimeout(
        this.connectToPierre(),
        connectionTimeoutMs,
        "proactive Dravr connection",
      );

      if (connectionResult === null) {
        // Connection timed out - this is non-fatal for the bridge
        this.log(
          `Proactive connection timed out after ${connectionTimeoutMs}ms - will connect on first tool use`,
        );
        this.log("Bridge will start with connect_to_dravr tool only");
        return;
      }

      // Cache tools immediately so they're ready for tools/list. Without a session
      // the server answers tools/list with its challenge, so the listing is not
      // attempted and the connect tool stands in until the user signs in.
      if (this.pierreClient && (await this.oauthProvider.tokens())) {
        const client = this.pierreClient;
        const toolsResult = await this.withTimeout(
          client.listTools(),
          toolsListTimeoutMs,
          "proactive tools list",
        );

        if (toolsResult) {
          this.cachedTools = toolsResult;
          this.log(
            `Cached ${toolsResult.tools.length} tools from Dravr: ${JSON.stringify(toolsResult.tools.map((t: any) => t.name))}`,
          );
        } else {
          this.log(
            `Tools list timed out after ${toolsListTimeoutMs}ms - will fetch on first request`,
          );
        }
      }
    } catch (error: any) {
      // If proactive connection fails, continue anyway
      // The bridge should still start - provide minimal toolset
      this.log(`Proactive connection failed: ${error.message}`);
      this.log("Bridge will start with connect_to_dravr tool only");
      // Don't propagate error - bridge should start successfully
    }
  }

  private async ensurePierreConnected(): Promise<void> {
    if (this.pierreClient) {
      return; // Already connected
    }

    const connectionTimeoutMs =
      this.config.toolCallConnectionTimeoutMs || 10000;
    this.log(
      `Connecting to Dravr MCP Server (timeout: ${connectionTimeoutMs}ms)...`,
    );

    const connectionResult = await this.withTimeout(
      this.connectToPierre(),
      connectionTimeoutMs,
      "tool-triggered Dravr connection",
    );

    if (connectionResult === null) {
      throw new PierreError(
        PierreErrorCode.TIMEOUT_ERROR,
        `Failed to connect to Dravr within ${connectionTimeoutMs}ms. Please use the "Connect to Dravr" tool to establish a connection.`,
      );
    }
  }

  private async connectToPierre(): Promise<void> {
    this.log("Connecting to Dravr MCP Server...");

    if (!this.oauthProvider) {
      throw new PierreError(
        PierreErrorCode.CONFIG_ERROR,
        "OAuth provider not initialized - call initializePierreConnection() first",
      );
    }

    this.log(`Target URL: ${this.mcpUrl}`);

    // Discovery runs with or without a session. Without one the server answers with
    // its RFC 9728 challenge, which still says the server is there and speaks the
    // modern revision; the toolset is then the connect tool until the user signs in.
    const existingTokens = await this.oauthProvider.tokens();
    if (existingTokens) {
      this.log("Found existing tokens - discovering with authentication");
    } else {
      this.log("No tokens found - discovering without a session");
    }

    await this.attemptConnection();
  }

  private async attemptConnection(): Promise<void> {
    if (!this.oauthProvider) {
      throw new PierreError(PierreErrorCode.CONFIG_ERROR, "OAuth provider not initialized");
    }
    const provider = this.oauthProvider;
    const maxAttempts = 3;

    for (let attempt = 1; ; attempt++) {
      // Await tokens() so async token loading completes: a synchronous savedTokens
      // check raced the load and read "no session" while one was being restored.
      const hasTokens = !!(await provider.tokens());
      this.log(
        hasTokens
          ? "Discovering Dravr MCP Server (authenticated)"
          : "Discovering Dravr MCP Server (no session yet)",
      );

      // There is no connection to open: revision 2026-07-28 is stateless, so the
      // client is a URL and a way to fetch the current bearer. Discovery is the one
      // request made up front, because it is how the client learns whether the server
      // serves the Tasks extension - the difference between a long tool call finishing
      // and it handing back "ask again later".
      const client = new McpHttpClient({
        url: this.mcpUrl,
        clientInfo: { name: "pierre-mcp-client", version: packageVersion },
        bearer: async () => (await provider.tokens())?.access_token,
        log: (message) => this.log(message),
      });

      try {
        const discovered = await client.discover();
        this.pierreClient = client;
        const server = discovered.serverInfo
          ? `${discovered.serverInfo.name} ${discovered.serverInfo.version}`
          : "Dravr MCP Server";
        this.log(
          `Connected to ${server} over MCP ${MCP_PROTOCOL_VERSION}` +
            (client.serverSupportsTasks() ? " (Tasks extension served)" : "") +
            (hasTokens ? "" : " - tool discovery only until you connect"),
        );
        return;
      } catch (error: any) {
        if (error instanceof McpHttpError && error.status === 401) {
          if (!hasTokens) {
            // The challenge is the server's whole answer to a request without a
            // session: reachable, modern, and waiting on a sign-in. The client stays
            // in place so the first tool call can start that sign-in.
            this.pierreClient = client;
            this.log(
              'Dravr MCP Server reachable; a session is needed for tools - use "Connect to Dravr"',
            );
            return;
          }
          if (attempt < maxAttempts) {
            this.log(
              `Stored session rejected, retrying... (attempt ${attempt}/${maxAttempts})`,
            );
            await provider.invalidateCredentials("tokens");
            await new Promise((resolve) => setTimeout(resolve, 1000));
            continue;
          }
          throw new PierreError(
            PierreErrorCode.AUTH_ERROR,
            `Dravr rejected the stored session ${maxAttempts} times - use "Connect to Dravr" to sign in again`,
          );
        }
        if (
          error instanceof McpRpcError &&
          error.code === RPC_UNSUPPORTED_PROTOCOL_VERSION
        ) {
          const supported = (error.data as { supported?: string[] } | undefined)
            ?.supported;
          throw new PierreError(
            PierreErrorCode.VALIDATION_ERROR,
            `Dravr MCP Server does not speak MCP ${MCP_PROTOCOL_VERSION}; it supports ${JSON.stringify(supported ?? [])}. Upgrade the server or use a client release matching it.`,
          );
        }
        this.log(`Failed to reach Dravr MCP Server: ${error?.message ?? error}`);
        throw error;
      }
    }
  }

  async initiateConnection(): Promise<void> {
    if (!this.oauthProvider) {
      throw new PierreError(PierreErrorCode.CONFIG_ERROR, "OAuth provider not initialized");
    }

    this.log("Initiating OAuth connection to Dravr MCP Server");

    // Check if we already have tokens
    const existingTokens = await this.oauthProvider.tokens();

    if (!existingTokens) {
      this.log("No tokens found - starting OAuth 2.0 authorization flow");

      // Manually trigger OAuth flow by building authorization URL and redirecting
      try {
        // Step 1: Ensure client is registered (dynamic client registration)
        let clientInfo = await this.oauthProvider.clientInformation();

        // Get client metadata for redirect URI (needed for both new and existing clients)
        const clientMetadata = this.oauthProvider["clientMetadata"];

        if (!clientInfo) {
          this.log(
            "No client info found - performing dynamic client registration",
          );

          // Generate new client credentials
          const crypto = require("crypto");
          const clientId = `pierre-bridge-${crypto.randomBytes(8).toString("hex")}`;
          const clientSecret = crypto.randomBytes(32).toString("hex");

          const fullClientInfo: OAuthClientInformationFull = {
            client_id: clientId,
            client_secret: clientSecret,
            redirect_uris: clientMetadata.redirect_uris,
            grant_types: clientMetadata.grant_types,
            response_types: clientMetadata.response_types,
            scope: clientMetadata.scope,
            client_name: clientMetadata.client_name,
            client_uri: clientMetadata.client_uri,
            client_id_issued_at: Math.floor(Date.now() / 1000),
            client_secret_expires_at: 0, // Never expires
          };

          // Save and register the client (this updates clientInfo with Dravr's assigned client_id)
          await this.oauthProvider.saveClientInformation(fullClientInfo);

          // Re-fetch client information to get the server-assigned client_id
          clientInfo = await this.oauthProvider.clientInformation();
          if (!clientInfo) {
            throw new PierreError(
              PierreErrorCode.AUTH_ERROR,
              "Client registration failed - no client info after registration",
            );
          }

          this.log(
            `Dynamic client registration complete: ${clientInfo.client_id}`,
          );
        }

        // Step 2: Get redirect URI
        const redirectUri = clientMetadata.redirect_uris[0];

        // Step 3: Generate PKCE values
        const state = await this.oauthProvider.state();
        const codeVerifier = this.oauthProvider.generateRandomString(64);
        await this.oauthProvider.saveCodeVerifier(codeVerifier);

        const codeChallenge =
          await this.oauthProvider.generateCodeChallenge(codeVerifier);

        // Step 4: Build authorization URL
        const authUrl = new URL(
          `${this.config.pierreServerUrl}/oauth2/authorize`,
        );
        authUrl.searchParams.set("client_id", clientInfo.client_id);
        authUrl.searchParams.set("redirect_uri", redirectUri);
        authUrl.searchParams.set("response_type", "code");
        authUrl.searchParams.set("state", state);
        authUrl.searchParams.set("code_challenge", codeChallenge);
        authUrl.searchParams.set("code_challenge_method", "S256");
        authUrl.searchParams.set("scope", "read:fitness write:fitness");

        // Step 5: Redirect to authorization (opens browser)
        await this.oauthProvider.redirectToAuthorization(authUrl);

        // Step 6: Connect after OAuth completes
        await this.attemptConnection();

        // Step 7: Refresh cached tools with authenticated toolset
        // Before OAuth, we may have cached unauthenticated tools (just connect_to_dravr)
        // After OAuth, we need to fetch and cache the FULL authenticated toolset
        try {
          if (this.pierreClient) {
            this.log("Fetching authenticated tools after OAuth...");
            const toolsResult = await this.pierreClient.listTools();
            this.cachedTools = toolsResult;
            this.log(
              `Refreshed cache with ${toolsResult.tools.length} authenticated tools: ${JSON.stringify(toolsResult.tools.map((t: any) => t.name))}`,
            );

            // Notify MCP host that tools have changed (now authenticated)
            if (this.mcpServer) {
              try {
                await this.mcpServer.notification({
                  method: "notifications/tools/list_changed",
                  params: {},
                });
                this.log("Sent tools/list_changed notification after OAuth");
              } catch (notifError: any) {
                this.log(
                  "Failed to send tools/list_changed notification:",
                  notifError.message,
                );
              }
            }
          }
        } catch (toolsError: any) {
          this.log("Failed to refresh tools after OAuth:", toolsError.message);
          // Non-fatal - tools will be fetched on next request
        }
      } catch (error) {
        this.log(`Failed to start OAuth flow: ${error}`);
        throw error;
      }
    } else {
      this.log(
        "Tokens already exist - connecting with existing authentication",
      );
      await this.attemptConnection();
    }

    this.log(
      `After attemptConnection, pierreClient is: ${!!this.pierreClient}`,
    );
  }

  /**
   * Starts the Dravr sign-in flow, or hands back the one already running.
   *
   * The flow outlives the wait on it: it owns the callback listener and its own
   * authorization deadline, and closes that listener itself when it settles - so a caller
   * that stops waiting leaves the browser page working and the tokens still landing in
   * storage. What a caller must never do is start a second flow on top of the first: that
   * replaces the code verifier the open page was challenged against, so neither page can
   * complete. Every entry point therefore joins the flow in flight.
   */
  private beginPierreLogin(): Promise<void> {
    if (!this.pierreLoginInFlight) {
      const login = this.initiateConnection().finally(() => {
        this.pierreLoginInFlight = null;
      });
      // The waits on this flow are bounded, so it can settle with nobody listening.
      // Record the outcome where it happens rather than losing it.
      login.then(
        () => this.log("Dravr sign-in flow completed"),
        (error: any) => this.log(`Dravr sign-in flow ended: ${error.message}`),
      );
      this.pierreLoginInFlight = login;
    }
    return this.pierreLoginInFlight;
  }

  /**
   * Waits for a sign-in flow, ending at the host's budget or at its cancellation -
   * whichever comes first - and reporting which of the three happened. Ending the wait
   * ends only the wait; the flow itself keeps running.
   */
  private async waitForPierreLogin(
    login: Promise<void>,
    waitMs: number,
    signal?: AbortSignal,
  ): Promise<"connected" | "pending" | "cancelled"> {
    if (signal?.aborted) {
      return "cancelled";
    }

    // ReturnType<typeof setTimeout> rather than NodeJS.Timeout: this package is
    // built inside a monorepo whose root node_modules carries DOM-flavoured
    // globals (@types/react-native, @types/jsdom), and when those win the
    // resolution setTimeout returns number. The inferred form is correct under
    // either.
    let budgetTimer: ReturnType<typeof setTimeout> | undefined;
    let onAbort: (() => void) | undefined;

    try {
      return await Promise.race([
        login.then(() => "connected" as const),
        new Promise<"pending">((resolve) => {
          budgetTimer = setTimeout(() => resolve("pending"), waitMs);
        }),
        new Promise<"cancelled">((resolve) => {
          onAbort = () => resolve("cancelled");
          signal?.addEventListener("abort", onAbort, { once: true });
        }),
      ]);
    } finally {
      clearTimeout(budgetTimer);
      if (onAbort) {
        signal?.removeEventListener("abort", onAbort);
      }
    }
  }

  /**
   * The budget an interactive browser wait gets inside the host's request, and the token
   * to report progress against. Both come from the same check because a progress token is
   * only usable with a channel to send it on: without one, nothing can hold off the
   * host's deadline, so the wait stays inside the default one.
   */
  private hostInteractiveBudget(extra?: any): {
    waitMs: number;
    progressToken: string | number | undefined;
  } {
    const progressToken =
      typeof extra?.sendNotification === "function"
        ? extra._meta?.progressToken
        : undefined;

    return {
      waitMs:
        progressToken === undefined
          ? INTERACTIVE_WAIT_MS
          : INTERACTIVE_PROGRESS_WAIT_MS,
      progressToken,
    };
  }

  /**
   * Reports progress for the duration of an interactive wait and returns the function
   * that stops it. The first report goes out immediately as well as on the interval, so
   * the host learns the call is waiting on a human before its first deadline gets close.
   */
  private reportInteractiveProgress(
    extra: any,
    progressToken: string | number | undefined,
    waitMs: number,
    message: string,
  ): () => void {
    if (progressToken === undefined) {
      return () => undefined;
    }

    const startedAt = Date.now();
    const sendProgress = () =>
      extra
        .sendNotification({
          method: "notifications/progress",
          params: {
            progressToken,
            progress: Date.now() - startedAt,
            total: waitMs,
            message,
          },
        })
        .catch((notifyError: any) =>
          this.log(`Failed to send progress notification: ${notifyError.message}`),
        );

    sendProgress();
    const progressTimer = setInterval(
      sendProgress,
      INTERACTIVE_PROGRESS_INTERVAL_MS,
    );
    return () => clearInterval(progressTimer);
  }

  /**
   * Forwards a tool call to Dravr and returns the tool's result, whether the server
   * answered inline or with a task handle the client then polled to completion.
   *
   * Progress reaches the host only when it offered a progressToken - that is the host's
   * opt-in, and the token every report must carry. The relayed count rises by one per
   * report, as the protocol requires, whatever the server's own progress values do. The
   * host's cancel signal travels with the call: on HTTP, closing the request is the
   * cancellation, and a task being polled is cancelled server-side as well.
   */
  private async callPierreTool(request: any, extra?: any): Promise<McpCallToolResult> {
    const client = this.pierreClient;
    if (!client) {
      throw new PierreError(PierreErrorCode.CONFIG_ERROR, "Not connected to Dravr");
    }
    const name: string = request.params.name;
    const hostToken =
      typeof extra?.sendNotification === "function"
        ? extra._meta?.progressToken
        : undefined;
    let reports = 0;
    const relay =
      hostToken === undefined
        ? undefined
        : (message: string) => {
            reports += 1;
            extra
              .sendNotification({
                method: "notifications/progress",
                params: { progressToken: hostToken, progress: reports, message },
              })
              .catch((notifyError: any) =>
                this.log(`Failed to relay progress: ${notifyError.message}`),
              );
          };

    return client.callTool(
      { name, arguments: request.params.arguments || {} },
      {
        signal: extra?.signal,
        onProgress: relay
          ? (progress: McpProgress) => relay(progress.message ?? `${name} in progress`)
          : undefined,
        onTask: (task: McpTask) => {
          this.log(
            `${name} answered with task ${task.taskId}; polling every ${task.pollIntervalMs ?? "default"} ms`,
          );
          relay?.(`${name} is running on the server (task ${task.taskId})`);
        },
        onTaskUpdate: (state: McpTaskState) => {
          relay?.(state.statusMessage ?? `${name}: ${state.status}`);
        },
      },
    );
  }

  /**
   * The cached catalogue as the host receives it: the tools alone. The cache hints
   * that come with a 2026-07-28 listing describe the server's freshness promise to
   * this bridge, not to the host on the far side of it.
   */
  private hostToolsList(): { tools: any[] } {
    return { tools: this.cachedTools.tools };
  }

  getClientSideTokenStatus(): {
    pierre: boolean;
    providers: Record<string, boolean>;
  } {
    if (!this.oauthProvider) {
      return { pierre: false, providers: {} };
    }

    return this.oauthProvider.getTokenStatus();
  }

  private async createMcpServer(): Promise<void> {
    this.log("Creating MCP host server...");

    // Create MCP server for MCP host
    this.mcpServer = new Server(
      {
        name: "pierre-fitness",
        version: "1.0.0",
      },
      {
        capabilities: {
          // listChanged is declared because the bridge emits
          // notifications/tools/list_changed: authenticating swaps the unauthenticated
          // toolset for the full one, and a host only re-fetches tools/list when the
          // capability was declared up front.
          tools: { listChanged: true },
          resources: {},
          prompts: {},
          logging: {},
          completions: {},
        },
      },
    );

    // Set up request handlers - bridge all requests to Dravr
    this.setupRequestHandlers();

    // Create stdio transport for MCP host
    this.serverTransport = new StdioServerTransport();

    this.log("MCP host server created");
  }

  private setupRequestHandlers(): void {
    if (!this.mcpServer) {
      throw new PierreError(PierreErrorCode.CONFIG_ERROR, "MCP server not initialized");
    }

    // Bridge tools/list requests
    this.mcpServer.setRequestHandler(
      ListToolsRequestSchema,
      async (_request) => {
        this.log("Bridging tools/list request");

        try {
          // Wait for proactive connection to complete if it's still running
          // This ensures we have the full toolset cached before responding
          // Use a shorter timeout (1 second) to avoid blocking tools/list too long
          if (this.proactiveConnectionPromise) {
            this.log("Waiting for proactive connection to complete...");
            const waitResult = await this.withTimeout(
              this.proactiveConnectionPromise,
              1000,
              "tools/list waiting for proactive connection",
            );

            // Clear the promise reference so subsequent calls don't wait
            this.proactiveConnectionPromise = null;

            if (waitResult === null) {
              this.log(
                "Proactive connection still running after 1s, proceeding with current cache",
              );
            } else {
              this.log("Proactive connection completed, checking cache");
            }
          }

          // If we have cached tools, return them immediately (from proactive connection)
          if (this.cachedTools) {
            this.log(
              `Using cached tools from proactive connection (${this.cachedTools.tools.length} tools)`,
            );
            return this.hostToolsList();
          }

          // Per MCP spec: tools/list MUST return ALL tools regardless of connection/auth status
          // If not connected yet, establish connection now (without auth is OK - server allows this)
          if (!this.pierreClient) {
            this.log(
              "Not connected - establishing connection to fetch tools (per MCP spec)",
            );
            try {
              await this.initializePierreConnection();
            } catch (error: any) {
              this.log(`Failed to connect to fetch tools: ${error.message}`);
              // Even if connection fails, we must return something
              // Return connect_to_dravr tool as fallback
              return {
                tools: [
                  {
                    name: "connect_to_dravr",
                    description:
                      "Connect to Dravr - Authenticate with Dravr Fitness Server to access your fitness data. This will open a browser window for secure login. Use this when you're not connected or need to reconnect.",
                    inputSchema: {
                      type: "object",
                      properties: {},
                      required: [],
                    },
                  },
                ],
              };
            }
          }

          // Now we should have a connection - fetch tools from server
          if (this.pierreClient) {
            this.log("Fetching tools from Dravr server");
            const client = this.pierreClient;
            const result = await client.listTools();
            this.log(`Received ${result.tools.length} tools from Dravr`);
            // Cache the result for next time
            this.cachedTools = result;
            return this.hostToolsList();
          }

          // Should not reach here, but safety fallback
          this.log("Unexpected: no Dravr client after connection attempt");
          return {
            tools: [
              {
                name: "connect_to_dravr",
                description:
                  "Connect to Dravr - Authenticate with Dravr Fitness Server to access your fitness data. This will open a browser window for secure login. Use this when you're not connected or need to reconnect.",
                inputSchema: {
                  type: "object",
                  properties: {},
                  required: [],
                },
              },
            ],
          };
        } catch (error: any) {
          this.log(`Error getting tools list: ${error.message || error}`);
          this.log("Providing connect tool only");

          return {
            tools: [
              {
                name: "connect_to_dravr",
                description:
                  "Connect to Dravr - Authenticate with Dravr Fitness Server to access your fitness data. This will open a browser window for secure login. Use this when you're not connected or need to reconnect.",
                inputSchema: {
                  type: "object",
                  properties: {},
                  required: [],
                },
              },
            ],
          };
        }
      },
    );

    // Bridge tools/call requests
    this.mcpServer.setRequestHandler(CallToolRequestSchema, async (request, extra) => {
      this.log("Bridging tool call:", request.params.name);

      // Handle special authentication tools
      if (request.params.name === "connect_to_dravr") {
        // extra carries the host's abort signal and progress token: signing in waits on
        // a human, so it needs both to stay inside the host's request budget.
        return await this.handleConnectToPierre(request, extra);
      }

      if (request.params.name === "connect_provider") {
        // extra carries the host's abort signal and progress token: connect_provider
        // waits on a human, so it needs both to stay inside the host's request budget.
        return await this.handleConnectProvider(request, extra);
      }

      // CRITICAL: Check for authentication tokens BEFORE attempting tool call
      // If no tokens, automatically trigger OAuth flow (not just return error)
      if (this.oauthProvider) {
        // IMPORTANT: Must await tokens() to ensure async token loading completes
        // Using synchronous savedTokens check causes race condition (tokens may not be loaded yet)
        const existingTokens = await this.oauthProvider.tokens();
        if (!existingTokens) {
          this.log(
            `No authentication tokens available - triggering OAuth flow for ${request.params.name}`,
          );

          // Start the sign-in rather than returning an error the user cannot act on:
          // the browser is the only place they can authenticate, and a host holding a
          // tool call open has no other way to send them there. The wait is bounded and
          // its reply names this tool, so a sign-in the budget did not outlive is
          // reported as what it is - still open, run this tool again - instead of
          // holding the call for five minutes the host stopped listening after.
          try {
            const connectResult = await this.handleConnectToPierre(request, extra);
            if (connectResult.isError || !(await this.oauthProvider.tokens())) {
              return connectResult;
            }
            // After successful OAuth, retry the original tool call
            this.log(`OAuth completed, retrying ${request.params.name}`);
          } catch (oauthError) {
            this.log(`OAuth flow failed: ${oauthError}`);
            return {
              content: [
                {
                  type: "text",
                  text: `Authentication required but OAuth flow failed: ${oauthError instanceof Error ? oauthError.message : String(oauthError)}. Please try again.`,
                },
              ],
              isError: true,
            };
          }
        }
      }

      // Ensure we have a connection before forwarding other tools
      try {
        await this.ensurePierreConnected();
      } catch (error) {
        return {
          content: [
            {
              type: "text",
              text: `Failed to connect to Dravr: ${error instanceof Error ? error.message : String(error)}. Please use the "Connect to Dravr" tool to authenticate.`,
            },
          ],
          isError: true,
        };
      }

      try {
        this.log(
          `Forwarding tool call ${request.params.name} to Dravr server...`,
        );
        const result = await this.callPierreTool(request, extra);
        this.log(
          `Tool call ${request.params.name} result:`,
          JSON.stringify(result).substring(0, 200),
        );

        // Validate response against Zod schema (logs warnings on mismatch, doesn't block)
        validateMcpToolResponse(request.params.name, result);

        return result;
      } catch (error) {
        this.log(`Tool call ${request.params.name} failed:`, error);

        // Check if this is an authentication error using multiple detection methods
        const errorAny = error as any;

        // Method 1: Check structured MCP error data (server sets authentication_failed: true)
        const authFailedFlag = errorAny?.data?.authentication_failed === true;

        // Method 2: Check MCP JSON-RPC error codes for auth errors
        const errorCode = errorAny?.code;
        const hasAuthErrorCode =
          errorCode && (errorCode === -32603 || errorCode === -32602);

        // Method 3: the transport's own verdict - a 401 carrying the RFC 9728 challenge
        const errorMessage =
          error instanceof Error ? error.message : String(error);
        const errorLower = errorMessage.toLowerCase();
        const hasHttpAuthStatus =
          error instanceof McpHttpError && error.status === 401;

        // Method 4: Check error message content
        const messageIndicatesAuth =
          errorLower.includes("unauthorized") ||
          errorLower.includes("authentication failed") ||
          errorLower.includes("jwt token") ||
          errorLower.includes("authentication") ||
          errorLower.includes("re-authenticate");

        const isAuthError =
          authFailedFlag ||
          hasAuthErrorCode ||
          hasHttpAuthStatus ||
          messageIndicatesAuth;

        if (isAuthError && this.oauthProvider) {
          this.log(
            `Authentication error detected - attempting automatic recovery`,
          );

          // Try to validate and refresh the token
          const tokens = await this.oauthProvider.tokens();
          if (tokens?.access_token && tokens?.refresh_token) {
            const validationResult = await this.oauthProvider.validateAndRefreshToken(
              tokens.access_token,
              tokens.refresh_token,
            );

            if (validationResult?.status === "refreshed") {
              this.log(`Session automatically renewed - retrying your request`);

              // Retry with the renewed session. validateAndRefreshToken adopted the new
              // pair before returning, and the transport asks the OAuth provider for
              // tokens on every send, so this retry carries the fresh access token.
              try {
                const retryResult = await this.callPierreTool(request, extra);
                this.log(`Request succeeded after automatic session renewal`);

                // Validate response against Zod schema
                validateMcpToolResponse(request.params.name, retryResult);

                return retryResult;
              } catch (retryError) {
                this.log(`Request failed even after session renewal`);
                return {
                  content: [
                    {
                      type: "text",
                      text: `Tool execution failed after token refresh: ${retryError instanceof Error ? retryError.message : String(retryError)}`,
                    },
                  ],
                  isError: true,
                };
              }
            } else if (validationResult?.status === "invalid") {
              this.log(`Automatic recovery failed - session cannot be renewed`);
              this.log(`Full re-authentication required`);

              // Clear the invalid connection
              await this.oauthProvider.invalidateCredentials("all");
              this.pierreClient = null;

              return {
                content: [
                  {
                    type: "text",
                    text: `Your session has expired and could not be refreshed. Please use the "Connect to Dravr" tool to re-authenticate.`,
                  },
                ],
                isError: true,
              };
            }
          }
        }

        // Return the original error if not an auth error or recovery failed
        return {
          content: [
            {
              type: "text",
              text: `Tool execution failed: ${errorMessage}`,
            },
          ],
          isError: true,
        };
      }
    });

    // Bridge resources/list requests
    this.mcpServer.setRequestHandler(
      ListResourcesRequestSchema,
      async (_request) => {
        this.log("Bridging resources/list request");

        // Dravr server doesn't provide resources, so always return empty list
        return { resources: [] };
      },
    );

    // Bridge resources/read requests
    this.mcpServer.setRequestHandler(
      ReadResourceRequestSchema,
      async (request) => {
        this.log("Bridging resource read:", request.params.uri);

        if (!this.pierreClient) {
          return {
            contents: [
              {
                type: "text",
                text: 'Not connected to Dravr. Please use the "Connect to Dravr" tool first to authenticate.',
              },
            ],
          };
        }

        return (await this.pierreClient.request(
          "resources/read",
          request.params,
        )) as any;
      },
    );

    // Bridge prompts/list requests
    this.mcpServer.setRequestHandler(
      ListPromptsRequestSchema,
      async (_request) => {
        this.log("Bridging prompts/list request");

        // Dravr server doesn't provide prompts, so always return empty list
        return { prompts: [] };
      },
    );

    // Bridge prompts/get requests
    this.mcpServer.setRequestHandler(
      GetPromptRequestSchema,
      async (request) => {
        this.log("Bridging prompt get:", request.params.name);

        if (!this.pierreClient) {
          return {
            description: "Not connected to Dravr",
            messages: [
              {
                role: "user",
                content: {
                  type: "text",
                  text: 'Not connected to Dravr. Please use the "Connect to Dravr" tool first to authenticate.',
                },
              },
            ],
          };
        }

        return (await this.pierreClient.request("prompts/get", request.params)) as any;
      },
    );

    // Handle ping requests
    this.mcpServer.setRequestHandler(PingRequestSchema, async () => {
      this.log("Handling ping request");
      return {};
    });

    // Handle logging/setLevel requests
    this.mcpServer.setRequestHandler(SetLevelRequestSchema, async (request) => {
      this.log(`Setting log level to: ${request.params.level}`);
      return {};
    });

    // Bridge completion requests
    this.mcpServer.setRequestHandler(CompleteRequestSchema, async (request) => {
      this.log("Bridging completion request");

      if (!this.pierreClient) {
        return {
          completion: {
            values: [],
            total: 0,
            hasMore: false,
          },
        };
      }

      return (await this.pierreClient.request(
        "completion/complete",
        request.params,
      )) as any;
    });

    this.log("Request handlers configured");
  }

  private async handleConnectToPierre(request: any, extra?: any): Promise<any> {
    // The retry instruction below names the tool the host actually called: the sign-in is
    // also started from connect_provider and from any tool call that finds no session, and
    // telling that caller to run connect_to_dravr would send it somewhere it never was.
    const toolName = request?.params?.name || "connect_to_dravr";

    try {
      this.log("Handling connect_to_dravr tool call - initiating OAuth flow");

      if (!this.oauthProvider) {
        return {
          content: [
            {
              type: "text",
              text: "OAuth provider not initialized. Please restart the bridge.",
            },
          ],
          isError: true,
        };
      }

      // Check if already authenticated
      // Credentials were validated at startup, so if they exist they're valid
      // IMPORTANT: Must await tokens() to ensure async token loading completes
      // Using synchronous savedTokens check causes race condition (tokens may not be loaded yet)
      const existingTokens = await this.oauthProvider.tokens();
      if (existingTokens && this.pierreClient) {
        return {
          content: [
            {
              type: "text",
              text: "Already connected to Dravr! You can now use all fitness tools to access your Strava and Fitbit data.",
            },
          ],
          isError: false,
        };
      }

      // CRITICAL: Refuse the interactive OAuth flow when it cannot complete.
      // Refuse if either:
      // 1. CI/CD (CI=true or GITHUB_ACTIONS=true) with no TTY — would hang automated tests, OR
      // 2. PIERRE_DISABLE_BROWSER=true — explicit kill switch for non-interactive runs (e.g.
      //    local/jest test suites) so they fail fast instead of repeatedly popping (and OOM-ing)
      //    a browser tab to /oauth2/login.
      // Real MCP hosts (Claude Code Desktop etc.) set neither, so OAuth still works there.
      const isCI =
        process.env.CI === "true" || process.env.GITHUB_ACTIONS === "true";
      const browserDisabled =
        process.env.PIERRE_DISABLE_BROWSER === "true" ||
        this.config.disableBrowser === true;
      const hasTTY = process.stdin.isTTY;

      if (!existingTokens && ((!hasTTY && isCI) || browserDisabled)) {
        this.log(
          "Refusing to start interactive OAuth flow in CI/CD environment (would hang automated tests)",
        );
        this.log(
          "Hint: In CI/CD, pre-authenticate using credentials or skip OAuth-requiring tests",
        );
        return {
          content: [
            {
              type: "text",
              text: "Authentication required but cannot start interactive OAuth flow in CI/CD environment. Please use credentials-based auth or skip OAuth tests.",
            },
          ],
          isError: true,
        };
      }

      // Wait for the sign-in inside the host's request budget. The flow keeps running
      // past this wait - see beginPierreLogin - so ending it early costs the user
      // nothing but the confirmation.
      const { waitMs, progressToken } = this.hostInteractiveBudget(extra);
      const stopProgress = this.reportInteractiveProgress(
        extra,
        progressToken,
        waitMs,
        "Waiting for Dravr sign-in in your browser",
      );

      let status: "connected" | "pending" | "cancelled";
      try {
        status = await this.waitForPierreLogin(
          this.beginPierreLogin(),
          waitMs,
          extra?.signal,
        );
      } finally {
        stopProgress();
      }

      if (status !== "connected") {
        // Not a failure: the sign-in page is still open, its callback still lands, and
        // the tokens are still stored when it does - the session is unconfirmed rather
        // than broken, and calling that a failure is what tells users something broke
        // while it is in fact completing. Nothing announces a finished Dravr sign-in to
        // the host, so the honest instruction is to finish in the browser and run the
        // tool again, not to wait for a message.
        this.log(
          `Dravr sign-in not confirmed within the host request budget (${status})`,
        );
        return {
          content: [
            {
              type: "text",
              text:
                status === "cancelled"
                  ? `Stopped waiting for Dravr sign-in, which is not confirmed yet. The page is still open in your browser - finish signing in there, then run ${toolName} again.`
                  : `Dravr sign-in is still open in your browser and is not confirmed yet.\n\n` +
                    `Finish signing in there, then run ${toolName} again. If you closed the page, running ${toolName} again starts a new sign-in.`,
            },
          ],
          isError: false,
        };
      }

      // Cache tools immediately after successful connection
      if (this.pierreClient) {
        try {
          const client = this.pierreClient;
          const tools = await client.listTools();
          this.cachedTools = tools;
          this.log(
            `Cached ${tools.tools.length} tools after connect_to_dravr: ${JSON.stringify(tools.tools.map((t: any) => t.name))}`,
          );
        } catch (toolError: any) {
          this.log(`Failed to cache tools: ${toolError.message}`);
        }
      }

      // Notify MCP host that tools have changed (now authenticated)
      if (this.mcpServer) {
        try {
          await this.mcpServer.notification({
            method: "notifications/tools/list_changed",
            params: {},
          });
          this.log("Sent tools/list_changed notification to MCP host");
        } catch (error: any) {
          this.log(
            "Failed to send tools/list_changed notification:",
            error.message,
          );
        }
      }

      return {
        content: [
          {
            type: "text",
            text:
              "Successfully connected to Dravr Fitness Server!\n\n" +
              "**Next step:** Connect to a fitness provider to access your activity data.\n\n" +
              "Available providers:\n" +
              "- **Strava** - Connect your Strava account to access activities, stats, and athlete profile\n" +
              "- **Fitbit** - Connect your Fitbit account (if you use Fitbit)\n\n" +
              'To connect to Strava, say: "Connect to Strava"',
          },
        ],
        isError: false,
      };
    } catch (error: any) {
      this.log("Failed to connect to Dravr:", error.message);

      return {
        content: [
          {
            type: "text",
            text: `Failed to connect to Dravr: ${error.message}. Please check that the Dravr server is running and try again.`,
          },
        ],
        isError: true,
      };
    }
  }

  private async handleConnectProvider(request: any, extra?: any): Promise<any> {
    try {
      this.log("Handling unified connect_provider tool call");

      if (!this.oauthProvider) {
        return {
          content: [
            {
              type: "text",
              text: "OAuth provider not initialized. Please restart the bridge.",
            },
          ],
          isError: true,
        };
      }

      // Extract provider from request parameters
      const provider = request.params.arguments?.provider || "strava";
      this.log(`Unified flow for provider: ${provider}`);

      // Step 1: Ensure Dravr authentication is complete. Signing in is itself an
      // interactive browser step, so it runs under the same bounded wait rather than
      // spending the host's whole budget before the provider flow has even started.
      if (!this.pierreClient) {
        this.log(
          "Dravr not connected - initiating Dravr authentication first",
        );
        const connectResult = await this.handleConnectToPierre(request, extra);
        if (connectResult.isError || !(await this.oauthProvider.tokens())) {
          return connectResult;
        }
        this.log("Dravr authentication completed");
      } else {
        this.log("Dravr already authenticated");
      }

      // Step 2: Check if provider is already connected
      this.log(`Checking if ${provider} is already connected`);
      try {
        if (this.pierreClient) {
          const connectionStatus = await this.pierreClient.callTool({
            name: "get_connection_status",
            arguments: { provider: provider },
          });

          // Check if the provider is already connected
          // The server returns structuredContent with providers array containing connection status
          if (connectionStatus) {
            this.log(
              `Full connection status response: ${JSON.stringify(connectionStatus).substring(0, 500)}...`,
            );

            // Access the structured content with provider connection status
            const structured = (connectionStatus as any).structuredContent;
            if (
              structured &&
              structured.providers &&
              Array.isArray(structured.providers)
            ) {
              const providerInfo = structured.providers.find(
                (p: any) =>
                  p.provider &&
                  p.provider.toLowerCase() === provider.toLowerCase(),
              );

              if (providerInfo && providerInfo.connected === true) {
                this.log(`${provider} is already connected - no OAuth needed`);
                return {
                  content: [
                    {
                      type: "text",
                      text: `Already connected to ${provider.toUpperCase()}! You can now access your ${provider} fitness data.`,
                    },
                  ],
                  isError: false,
                };
              } else {
                this.log(
                  `${provider} connected status: ${providerInfo ? providerInfo.connected : "not found"}`,
                );
              }
            }
          }
        }

        this.log(`${provider} not connected - proceeding with OAuth flow`);
      } catch (error: any) {
        this.log(
          `Could not check connection status: ${error.message} - proceeding with OAuth anyway`,
        );
      }

      // Step 3: Extract user_id from JWT token
      const tokens = await this.oauthProvider.tokens();
      if (!tokens?.access_token) {
        throw new PierreError(PierreErrorCode.AUTH_ERROR, "No access token available");
      }

      // Decode JWT to get user_id (JWT format: header.payload.signature)
      const payload = tokens.access_token.split(".")[1];
      const decoded = JSON.parse(Buffer.from(payload, "base64").toString());
      const userId = decoded.sub;

      if (!userId) {
        throw new PierreError(PierreErrorCode.AUTH_ERROR, "Could not extract user_id from JWT token");
      }

      this.log(`Initiating ${provider} OAuth flow for user: ${userId}`);

      // Ensure callback server is running to receive provider OAuth completion notification
      // The server will POST to this callback when provider OAuth completes
      if (this.oauthProvider) {
        const oauthProviderAny = this.oauthProvider as any;
        if (!oauthProviderAny.callbackServer) {
          this.log("Starting callback server for provider OAuth notification");
          // Accessing redirectUrl triggers startCallbackServerSync internally
          const callbackUrl = oauthProviderAny.redirectUrl;
          this.log(`Callback server ready at ${callbackUrl}`);
        }
      }

      try {
        // Correct OAuth URL format: /api/oauth/auth/{provider}/{user_id}
        const providerOAuthUrl = `${this.config.pierreServerUrl}/api/oauth/auth/${provider}/${userId}`;

        // Open provider OAuth in browser with focus
        openUrlInBrowserWithFocus(providerOAuthUrl, {
          disableBrowser: this.config.disableBrowser,
          log: (message) => this.log(message),
        });

        this.log(`Opened ${provider} OAuth in browser: ${providerOAuthUrl}`);
        this.log(`Waiting for ${provider} OAuth to complete...`);

        // Wait for provider OAuth to complete, inside the host's request budget and
        // under its cancellation. Whatever the outcome here, the browser flow keeps
        // running and announces itself through notifications/message when it lands.
        const { waitMs, progressToken } = this.hostInteractiveBudget(extra);
        const stopProgress = this.reportInteractiveProgress(
          extra,
          progressToken,
          waitMs,
          `Waiting for ${provider} authorization in your browser`,
        );

        try {
          await this.oauthProvider.waitForProviderOAuth(
            provider,
            waitMs,
            extra?.signal,
          );
        } finally {
          stopProgress();
        }

        this.log(`${provider} OAuth completed successfully`);

        const capitalizedProvider =
          provider.charAt(0).toUpperCase() + provider.slice(1);

        return {
          content: [
            {
              type: "text",
              text: `${capitalizedProvider} connected successfully!\n\nYou now have full access to your ${capitalizedProvider} fitness data. Try asking me about your recent activities, stats, or training insights!`,
            },
          ],
          isError: false,
        };
      } catch (error: any) {
        if (extra?.signal?.aborted) {
          this.log(
            `${provider} OAuth wait ended because the host cancelled the request`,
          );
          return {
            content: [
              {
                type: "text",
                text: `Stopped waiting for ${provider.toUpperCase()} authorization. The page is still open in your browser - finishing there completes the connection.`,
              },
            ],
            isError: false,
          };
        }

        if (error.code === PierreErrorCode.TIMEOUT_ERROR) {
          // Not a failure. The authorization page is still open and its callback still
          // arrives whenever the user finishes, so the connection is unconfirmed rather
          // than broken - reporting it as failed is what told users a connection had
          // failed while it was in fact completing.
          this.log(
            `${provider} OAuth not confirmed within the host request budget - reporting it as still pending`,
          );
          return {
            content: [
              {
                type: "text",
                text:
                  `${provider.toUpperCase()} authorization is still open in your browser and is not confirmed yet.\n\n` +
                  `Finish signing in there and you will get a confirmation message as soon as it completes - then ask for your ${provider} data. ` +
                  `If you closed the page, run connect_provider again.`,
              },
            ],
            isError: false,
          };
        }
        this.log(`Failed to complete ${provider} OAuth: ${error.message}`);
        return {
          content: [
            {
              type: "text",
              text: `Dravr authentication successful, but failed to open ${provider.toUpperCase()} OAuth: ${error.message}. You can manually visit the OAuth page in Dravr's web interface.`,
            },
          ],
          isError: false, // Not a complete failure since Dravr auth worked
        };
      }
    } catch (error: any) {
      this.log("Unified connect_provider failed:", error.message);

      return {
        content: [
          {
            type: "text",
            text: `Unified authentication failed: ${error.message}. Please check that Dravr server is running and try again.`,
          },
        ],
        isError: true,
      };
    }
  }

  private async startBridge(): Promise<void> {
    if (!this.mcpServer || !this.serverTransport) {
      throw new PierreError(PierreErrorCode.CONFIG_ERROR, "Server or transport not initialized");
    }

    // Install batch request guard on transport
    installBatchGuard(this.serverTransport, this.log.bind(this));

    // Start the stdio server for MCP host
    await this.mcpServer.connect(this.serverTransport);

    // IMPORTANT: Intercept messages AFTER connect() to ensure our handler isn't overwritten
    // The Server.connect() sets up its own onmessage handler, so we need to wrap it
    const mcpServerOnMessage = this.serverTransport.onmessage;
    this.serverTransport.onmessage = createBatchGuardMessageHandler(
      this.serverTransport,
      mcpServerOnMessage,
      this.log.bind(this),
    );

    this.log(
      "Bridge is running - MCP host can now access Dravr Fitness tools",
    );
  }

  async stop(): Promise<void> {
    this.log("Stopping bridge...");

    try {
      // Close Dravr client connection
      if (this.pierreClient) {
        this.pierreClient.close();
        this.pierreClient = null;
      }

      // Close MCP server
      if (this.mcpServer) {
        await this.mcpServer.close();
        this.mcpServer = null;
      }

      // Close OAuth callback server
      if (this.oauthProvider && (this.oauthProvider as any).callbackServer) {
        const callbackServer = (this.oauthProvider as any).callbackServer;
        return new Promise<void>((resolve) => {
          callbackServer.close(() => {
            this.log("OAuth callback server closed");
            resolve();
          });
        });
      }

      this.log("Bridge stopped");
    } catch (error) {
      this.log("Error stopping bridge:", error);
      throw error;
    }
  }
}
