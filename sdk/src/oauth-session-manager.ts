// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: OAuth 2.0 session management for Dravr MCP Client
// ABOUTME: Handles token storage, refresh, client registration, and callback server for OAuth flows

import { OAuthClientProvider } from "@modelcontextprotocol/sdk/client/auth.js";
import { readFileSync } from "fs";
import { join } from "path";
import {
  OAuthClientMetadata,
  OAuthClientInformation,
  OAuthTokens,
  OAuthClientInformationFull,
} from "@modelcontextprotocol/sdk/shared/auth.js";
import { randomBytes, createHash, timingSafeEqual } from "node:crypto";
import { createSecureStorage, SecureTokenStorage } from "./secure-storage.js";
import { openUrlInBrowserWithFocus } from "./browser-launcher.js";
import { PierreError, PierreErrorCode } from "./errors.js";

// Load OAuth HTML templates from dist/templates/ (copied during build)
// Templates are self-contained in the SDK bundle for portability
const OAUTH_SUCCESS_TEMPLATE = readFileSync(
  join(__dirname, "templates/oauth_success.html"),
  "utf-8",
);
const OAUTH_ERROR_TEMPLATE = readFileSync(
  join(__dirname, "templates/oauth_error.html"),
  "utf-8",
);

/** Callback port published in the redirect URI when the caller does not choose one */
const DEFAULT_CALLBACK_PORT = 35535;

/** How long the flow waits for the user to finish authorizing in the browser */
const DEFAULT_AUTHORIZATION_TIMEOUT_MS = 300000;

/** Accepted shape of the {provider} segment of the provider token callback path */
const PROVIDER_CALLBACK_PATH = /^\/oauth\/provider-callback\/([A-Za-z0-9_-]{1,32})$/;

/**
 * Compares two secrets without leaking their contents through timing. Lengths are
 * compared first because timingSafeEqual throws on unequal buffer lengths; that only
 * reveals the length of a presented value, never how far it matched.
 */
function constantTimeEquals(presented: string, expected: string): boolean {
  const presentedBytes = Buffer.from(presented, "utf8");
  const expectedBytes = Buffer.from(expected, "utf8");
  if (presentedBytes.length !== expectedBytes.length) {
    return false;
  }
  return timingSafeEqual(presentedBytes, expectedBytes);
}

/** Stored tokens structure for keychain persistence */
export interface StoredTokens {
  pierre?: OAuthTokens & { saved_at?: number };
  providers?: Record<
    string,
    {
      access_token: string;
      refresh_token?: string;
      expires_at?: number;
      token_type?: string;
      scope?: string;
    }
  >;
  // OAuth client registration info (includes client_secret) - stored securely alongside tokens
  client_info?: OAuthClientInformationFull;
}

/** Base configuration shared by all authentication modes */
export interface OAuthSessionConfigBase {
  pierreServerUrl: string;
  callbackPort?: number;
  disableBrowser?: boolean;
  tokenValidationTimeoutMs?: number;
  /** Upper bound on the wait for the browser authorization callback (default 5 minutes) */
  authorizationTimeoutMs?: number;
}

/** JWT token authentication mode */
export interface OAuthSessionConfigJwt extends OAuthSessionConfigBase {
  mode: 'jwt';
  jwtToken: string;
}

/** OAuth 2.0 client credentials authentication mode */
export interface OAuthSessionConfigOAuth extends OAuthSessionConfigBase {
  mode: 'oauth';
  oauthClientId: string;
  oauthClientSecret: string;
}

/** API key authentication mode */
export interface OAuthSessionConfigApiKey extends OAuthSessionConfigBase {
  mode: 'api-key';
  apiKey: string;
}

/** Discriminated union for authentication modes */
export type OAuthSessionConfig = OAuthSessionConfigJwt | OAuthSessionConfigOAuth | OAuthSessionConfigApiKey;

interface OAuth2TokenResponse {
  access_token: string;
  token_type: string;
  expires_in?: number;
  refresh_token?: string;
  scope?: string;
}

interface ValidateRefreshResponse {
  status: "valid" | "refreshed" | "invalid";
  expires_in?: number;
  access_token?: string;
  refresh_token?: string;
  token_type?: string;
  reason?: string;
  requires_full_reauth?: boolean;
}

export class PierreOAuthClientProvider implements OAuthClientProvider {
  private serverUrl: string;
  private config: OAuthSessionConfig;
  private clientInfo: OAuthClientInformationFull | undefined = undefined;
  private savedTokens: OAuthTokens | undefined = undefined;
  private codeVerifierValue: string | undefined = undefined;
  private stateValue: string | undefined = undefined;
  private callbackServer: any = undefined;
  private authorizationPending: Promise<any> | undefined = undefined;
  private callbackPort: number = 0;
  private callbackSessionToken: string | undefined = undefined;
  private callbackServerReady: Promise<number> | undefined = undefined;
  private callbackBindError: PierreError | undefined = undefined;
  private refreshInFlight: Promise<void> | undefined = undefined;

  // Secure token storage using OS keychain
  private secureStorage: SecureTokenStorage | undefined = undefined;
  private allStoredTokens: StoredTokens = {};

  // Callback for notifying when provider OAuth completes (called by PierreMcpClient)
  private onProviderOAuthComplete:
    | ((provider: string) => Promise<void>)
    | undefined;

  // Pending provider OAuth promises (keyed by provider name)
  private pendingProviderOAuth: Map<
    string,
    { resolve: (value: any) => void; reject: (error: any) => void }
  > = new Map();

  constructor(
    serverUrl: string,
    config: OAuthSessionConfig,
    onProviderOAuthComplete?: (provider: string) => Promise<void>,
  ) {
    this.onProviderOAuthComplete = onProviderOAuthComplete;
    this.serverUrl = serverUrl;
    this.config = config;

    // NOTE: Secure storage initialization is async, so it's deferred to start()
    // to avoid race conditions with constructor completion
    // See initializePierreConnection() for the actual initialization
    // Client info (including client_secret) is loaded from secure storage alongside tokens

    this.log(`OAuth client provider created for server: ${serverUrl}`);
    this.log(
      `Using OS keychain for secure token storage (will initialize on start)`,
    );
  }

  /**
   * Waits for provider OAuth to complete (called from
   * PierreMcpClient.handleConnectProvider).
   *
   * The optional signal is the MCP request's abort signal: when the host abandons the
   * tool call, the wait ends with it rather than blocking on a request nobody is
   * listening to any more. Every exit - completion, timeout, cancellation - clears the
   * timer and drops the pending entry, so an abandoned wait holds nothing open. The
   * browser flow itself keeps running: whenever it lands, the callback stores the
   * provider token and announces it through onProviderOAuthComplete.
   */
  public waitForProviderOAuth(
    provider: string,
    timeoutMs: number = 120000,
    signal?: AbortSignal,
  ): Promise<void> {
    return new Promise((resolve, reject) => {
      this.log(`Waiting for ${provider} OAuth completion (timeout: ${timeoutMs}ms)`);

      if (signal?.aborted) {
        reject(
          new PierreError(
            PierreErrorCode.PROVIDER_ERROR,
            `${provider} OAuth wait cancelled by the MCP host`,
          ),
        );
        return;
      }

      const settle = (finish: () => void) => {
        clearTimeout(timeoutId);
        signal?.removeEventListener("abort", onAbort);
        this.pendingProviderOAuth.delete(provider);
        finish();
      };

      const onAbort = () =>
        settle(() => {
          this.log(`${provider} OAuth wait cancelled by the MCP host`);
          reject(
            new PierreError(
              PierreErrorCode.PROVIDER_ERROR,
              `${provider} OAuth wait cancelled by the MCP host`,
            ),
          );
        });

      const timeoutId = setTimeout(
        () =>
          settle(() =>
            reject(
              new PierreError(
                PierreErrorCode.TIMEOUT_ERROR,
                `${provider} OAuth timed out after ${timeoutMs}ms`,
              ),
            ),
          ),
        timeoutMs,
      );

      signal?.addEventListener("abort", onAbort, { once: true });

      this.pendingProviderOAuth.set(provider, {
        resolve: (value: any) => settle(() => resolve(value)),
        reject: (error: any) => settle(() => reject(error)),
      });
    });
  }

  // Resolve pending provider OAuth (called from callback handler)
  private resolveProviderOAuth(provider: string): void {
    const pending = this.pendingProviderOAuth.get(provider);
    if (pending) {
      this.log(`Resolving ${provider} OAuth promise`);
      pending.resolve({ provider });
      this.pendingProviderOAuth.delete(provider);
    }
  }

  public async initializeSecureStorage(): Promise<void> {
    try {
      this.secureStorage = await createSecureStorage(this.log.bind(this));
      // Load existing tokens and client info from keychain
      await this.loadStoredTokens();

      // Load client info from secure storage (stored alongside tokens)
      this.loadClientInfoFromSecureStorage();

      // Migrate legacy plaintext client info file to secure storage
      await this.migratePlaintextClientInfo();

      // If a JWT was supplied through PIERRE_JWT_TOKEN, use it for authentication.
      // CRITICAL: that token ALWAYS takes precedence over keychain tokens
      if (this.config.mode === 'jwt') {
        this.log(
          "Using JWT token from the environment for authentication (overrides keychain)",
        );
        this.savedTokens = {
          access_token: this.config.jwtToken,
          token_type: "Bearer",
          expires_in: 3600, // Default 1 hour, actual expiry is in the JWT itself
          scope: "read:fitness write:fitness",
          // Note: No refresh_token when using direct JWT
        };
        this.log("JWT token loaded from configuration");
      }

      this.log("Secure storage initialized with OS keychain");
    } catch (error) {
      this.log(`Failed to initialize secure storage: ${error}`);
      this.log("WARNING: Token storage will not be available");
    }
  }

  private async migratePlaintextClientInfo(): Promise<void> {
    const os = require("os");
    const path = require("path");
    const fs = require("fs");
    const legacyPath = path.join(os.homedir(), ".pierre-mcp-client-info.json");

    try {
      if (!fs.existsSync(legacyPath)) {
        return;
      }

      // Skip if we already have client info in secure storage
      if (this.clientInfo) {
        this.log("Client info already in secure storage, removing legacy plaintext file");
        fs.unlinkSync(legacyPath);
        return;
      }

      this.log(`Migrating plaintext client info from ${legacyPath} to secure storage`);
      const data = fs.readFileSync(legacyPath, "utf8");
      this.clientInfo = JSON.parse(data);

      // Save to secure storage
      await this.saveClientInfo();

      // Verify migration succeeded
      if (this.allStoredTokens.client_info) {
        fs.unlinkSync(legacyPath);
        this.log("Successfully migrated client info to secure storage and deleted plaintext file");
      } else {
        this.log("Migration verification failed - keeping plaintext file");
      }
    } catch (error) {
      this.log(`Failed to migrate plaintext client info: ${error}`);
    }
  }

  private log(message: string, ...args: any[]): void {
    const timestamp = new Date().toISOString();
    console.error(`[${timestamp}] [Dravr OAuth] ${message}`, ...args);
  }

  // Client-side token storage methods using secure keychain
  private async loadStoredTokens(): Promise<void> {
    try {
      if (!this.secureStorage) {
        this.log("Secure storage not initialized, skipping token load");
        return;
      }

      const tokens = await this.secureStorage.getTokens();
      if (tokens) {
        this.allStoredTokens = tokens;

        // Load Dravr tokens into memory for MCP SDK compatibility
        if (this.allStoredTokens.pierre) {
          this.savedTokens = this.allStoredTokens.pierre;
          this.log(`Loaded Dravr tokens from keychain`);
        }

        if (this.allStoredTokens.providers) {
          const providerCount = Object.keys(
            this.allStoredTokens.providers,
          ).length;
          this.log(`Loaded ${providerCount} provider token(s) from keychain`);
        }
      } else {
        this.log(`No stored tokens found in keychain, starting fresh`);
      }
    } catch (error) {
      this.log(`Failed to load stored tokens: ${error}`);
      this.allStoredTokens = {};
    }
  }

  private loadClientInfoFromSecureStorage(): void {
    // Load client info from the secure token storage (already loaded via loadStoredTokens)
    if (this.allStoredTokens.client_info) {
      this.clientInfo = this.allStoredTokens.client_info;
      this.log(
        `Loaded client info from secure storage: ${this.clientInfo?.client_id}`,
      );
    } else {
      this.log(
        `No stored client info found, will perform dynamic registration on first OAuth`,
      );
    }
  }

  private async saveClientInfo(): Promise<void> {
    if (!this.clientInfo) {
      return;
    }

    try {
      this.allStoredTokens.client_info = this.clientInfo;
      await this.saveStoredTokens();
      this.log(`Saved client info to secure storage: ${this.clientInfo.client_id}`);
    } catch (error) {
      this.log(`Failed to save client info: ${error}`);
    }
  }

  private async clearClientInfo(): Promise<void> {
    this.clientInfo = undefined;
    delete this.allStoredTokens.client_info;
    await this.saveStoredTokens();
    this.log("Cleared client info from secure storage");
  }

  private async saveStoredTokens(): Promise<void> {
    try {
      if (!this.secureStorage) {
        this.log("Secure storage not initialized, cannot save tokens");
        return;
      }

      await this.secureStorage.saveTokens(this.allStoredTokens);
      this.log(`Saved tokens to OS keychain`);
    } catch (error) {
      this.log(`Failed to save tokens to keychain: ${error}`);
    }
  }

  async clearTokens(): Promise<void> {
    try {
      // Clear in-memory Dravr tokens
      this.savedTokens = undefined;
      delete this.allStoredTokens.pierre;

      // Clear from keychain if secure storage is available
      if (this.secureStorage) {
        await this.secureStorage.clearTokens();
        this.log(`Cleared all tokens from keychain`);
      }

      // Reset in-memory storage
      this.allStoredTokens = {};
    } catch (error) {
      this.log(`Failed to clear tokens: ${error}`);
    }
  }

  async saveProviderToken(provider: string, tokenData: any): Promise<void> {
    if (!this.allStoredTokens.providers) {
      this.allStoredTokens.providers = {};
    }

    this.allStoredTokens.providers[provider] = {
      access_token: tokenData.access_token,
      refresh_token: tokenData.refresh_token,
      expires_at: tokenData.expires_in
        ? Date.now() + tokenData.expires_in * 1000
        : undefined,
      token_type: tokenData.token_type || "Bearer",
      scope: tokenData.scope,
    };

    await this.saveStoredTokens();
    this.log(`Saved ${provider} provider token to client storage`);
  }

  getProviderToken(provider: string): any | undefined {
    const token = this.allStoredTokens.providers?.[provider];
    if (!token) {
      return undefined;
    }

    // Check if token is expired
    if (token.expires_at && Date.now() > token.expires_at) {
      this.log(`${provider} token expired, removing from storage`);
      delete this.allStoredTokens.providers![provider];
      this.saveStoredTokens();
      return undefined;
    }

    return token;
  }

  get redirectUrl(): string {
    // Ensure callback server is started before providing redirect URL
    if (this.callbackPort === 0 && !this.callbackServer) {
      // Start callback server synchronously if not already started
      this.startCallbackServerSync();
    }
    // A port that failed to bind is never published: the URI would name a port held by
    // another process. Callers await ensureCallbackServerBound() before sending it.
    if (this.callbackBindError) {
      throw this.callbackBindError;
    }
    // Wait for callbackPort to be set by the server startup
    if (this.callbackPort === 0) {
      throw new PierreError(PierreErrorCode.CONFIG_ERROR, "Callback server failed to start - no port available");
    }
    return `http://localhost:${this.callbackPort}/oauth/callback`;
  }

  get clientMetadata(): OAuthClientMetadata {
    return {
      client_name: "Dravr MCP Client",
      client_uri: "https://claude.ai",
      redirect_uris: [this.redirectUrl],
      grant_types: ["authorization_code"],
      response_types: ["code"],
      scope: "read:fitness write:fitness",
      token_endpoint_auth_method: "client_secret_basic",
    };
  }

  async state(): Promise<string> {
    if (!this.stateValue) {
      this.stateValue = this.generateRandomString(32);
    }
    return this.stateValue;
  }

  async clientInformation(): Promise<OAuthClientInformation | undefined> {
    if (this.config.mode === 'oauth') {
      return {
        client_id: this.config.oauthClientId,
        client_secret: this.config.oauthClientSecret,
      };
    }
    return this.clientInfo
      ? {
          client_id: this.clientInfo.client_id,
          client_secret: this.clientInfo.client_secret,
        }
      : undefined;
  }

  async saveClientInformation(
    clientInformation: OAuthClientInformationFull,
  ): Promise<void> {
    this.log(
      `Registering client with Dravr OAuth server: ${clientInformation.client_id}`,
    );

    // Register this client with Dravr's OAuth server
    await this.registerClientWithPierre(clientInformation);

    this.clientInfo = clientInformation;
    await this.saveClientInfo();
    this.log(
      `Saved client info to secure storage: ${clientInformation.client_id}`,
    );
  }

  private async registerClientWithPierre(
    clientInfo: OAuthClientInformationFull,
  ): Promise<void> {
    const registrationEndpoint = `${this.serverUrl}/oauth2/register`;

    // The registration request publishes the redirect URI, so the callback port must
    // already be held by this process when it goes out.
    await this.ensureCallbackServerBound();

    const registrationRequest = {
      client_id: clientInfo.client_id,
      client_secret: clientInfo.client_secret,
      redirect_uris: this.clientMetadata.redirect_uris,
      grant_types: this.clientMetadata.grant_types,
      response_types: this.clientMetadata.response_types,
      scope: this.clientMetadata.scope,
      client_name: this.clientMetadata.client_name,
      client_uri: this.clientMetadata.client_uri,
    };

    this.log(`Registering client at ${registrationEndpoint}`);

    try {
      // Use native fetch (Node 24+)
      const response = await fetch(registrationEndpoint, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Accept: "application/json",
        },
        body: JSON.stringify(registrationRequest),
      });

      if (!response.ok) {
        const errorText = await response.text();
        throw new PierreError(
          PierreErrorCode.AUTH_ERROR,
          `Client registration failed: ${response.status} ${response.statusText}: ${errorText}`,
        );
      }

      const registrationResponse: any = await response.json();

      // Security: Mask client_secret before logging
      const sanitizedResponse = { ...registrationResponse };
      if (sanitizedResponse.client_secret) {
        sanitizedResponse.client_secret = "***REDACTED***";
      }
      this.log(
        `Client registration successful: ${JSON.stringify(sanitizedResponse)}`,
      );

      // Update client info with the response from Dravr's server
      if (
        registrationResponse.client_id &&
        registrationResponse.client_secret
      ) {
        this.log(
          `Updating client info to use Dravr's returned client ID: ${registrationResponse.client_id}`,
        );
        clientInfo.client_id = registrationResponse.client_id;
        clientInfo.client_secret = registrationResponse.client_secret;
      }
    } catch (error) {
      this.log(`Client registration failed: ${error}`);
      throw error;
    }
  }

  async saveTokens(tokens: OAuthTokens): Promise<void> {
    this.savedTokens = tokens;

    // Also save to persistent client-side storage
    this.allStoredTokens.pierre = {
      ...tokens,
      saved_at: Math.floor(Date.now() / 1000),
    };
    await this.saveStoredTokens();

    this.log(`Saved Dravr tokens: expires_in=${tokens.expires_in}`);
  }

  async tokens(): Promise<OAuthTokens | undefined> {
    // If no in-memory tokens, try to load from persistent storage
    if (!this.savedTokens && this.allStoredTokens.pierre) {
      // Single-flight. A refresh rotates the refresh token, so a second concurrent
      // attempt presents one the server has already retired and is told "invalid" -
      // and that verdict used to delete the session the first attempt had just renewed.
      // Concurrent callers await the one attempt and share its outcome.
      if (!this.refreshInFlight) {
        this.refreshInFlight = this.reloadTokensFromStorage().finally(() => {
          this.refreshInFlight = undefined;
        });
      }
      await this.refreshInFlight;
    }

    this.log(
      `tokens() called, returning tokens: ${this.savedTokens ? "available" : "none"}`,
    );
    if (this.savedTokens) {
      this.log(`Token type: ${this.savedTokens.token_type}`);
    }
    return this.savedTokens;
  }

  private async reloadTokensFromStorage(): Promise<void> {
    const storedTokens = this.allStoredTokens.pierre;
    if (!storedTokens) {
      return;
    }

    this.log(
      `No in-memory tokens, attempting to reload from persistent storage`,
    );

    // Always validate with server using validate-and-refresh endpoint
    const validationResult = await this.validateAndRefreshToken(
      storedTokens.access_token,
      storedTokens.refresh_token,
    );

    if (!validationResult) {
      // The request itself did not complete (network failure or timeout), which says
      // nothing about whether the session is still good. The stored credentials stay
      // exactly as they are so a later call can retry; discarding them here would sign
      // the user out on a blip and leave memory and keychain disagreeing.
      this.log(`Unable to validate your session with Dravr server`);
      this.log(`Keeping stored credentials - the next call retries validation`);
      return;
    }

    if (validationResult.status === "valid") {
      // Token is valid, reload into memory
      this.savedTokens = {
        access_token: storedTokens.access_token,
        refresh_token: storedTokens.refresh_token,
        expires_in: storedTokens.expires_in,
        token_type: storedTokens.token_type || "Bearer",
        scope: storedTokens.scope,
      };
      this.log(
        `Your session is valid (expires in ${validationResult.expires_in}s)`,
      );
      return;
    }

    if (validationResult.status === "refreshed") {
      // validateAndRefreshToken has already adopted the renewed pair into memory and
      // secure storage, so there is nothing left to write here.
      this.log(
        `Session renewed successfully - you can continue using Dravr tools`,
      );
      return;
    }

    // Token is invalid, clear storage and require full re-auth. Memory and keychain are
    // cleared together so no caller is left holding half of a dead session.
    this.log(`Your session has expired and cannot be renewed automatically`);
    this.log(`Reason: ${validationResult.reason || "Token validation failed"}`);
    this.log(`Please use the "Connect to Dravr" tool to re-authenticate`);
    this.savedTokens = undefined;
    delete this.allStoredTokens.pierre;
    await this.saveStoredTokens();
  }

  /**
   * Adopts the pair a refresh just minted, into memory and secure storage.
   *
   * This is owned by validateAndRefreshToken rather than by its callers because the
   * server rotates the refresh token: the request that renews the session consumes the
   * stored refresh token, so a caller that returned without writing would leave the
   * keychain holding a token the server has already retired and an access token that has
   * already been rejected. The next validation calls that session invalid and signs the
   * user out. Persisting here makes every caller correct without having to know that.
   */
  private async adoptRefreshedTokens(
    result: ValidateRefreshResponse,
  ): Promise<void> {
    if (!result.access_token) {
      this.log(
        `Refresh reported success without an access token - keeping the stored session`,
      );
      return;
    }

    // The scope is not echoed by the refresh response; it belongs to the session being
    // renewed, so it is carried over from whichever copy of that session is loaded.
    const scope = this.savedTokens?.scope ?? this.allStoredTokens.pierre?.scope;

    this.savedTokens = {
      access_token: result.access_token,
      refresh_token: result.refresh_token,
      expires_in: result.expires_in,
      token_type: result.token_type || "Bearer",
      scope,
    };

    this.allStoredTokens.pierre = {
      ...this.savedTokens,
      saved_at: Math.floor(Date.now() / 1000),
    };
    await this.saveStoredTokens();

    this.log(`Your session was automatically renewed with a fresh token`);
  }

  public async validateAndRefreshToken(
    accessToken: string,
    refreshToken?: string,
  ): Promise<ValidateRefreshResponse | null> {
    try {
      this.log(`Validating token with server validate-and-refresh endpoint`);

      const timeoutMs = this.config.tokenValidationTimeoutMs || 3000;

      const fetchPromise = fetch(
        `${this.serverUrl}/oauth2/validate-and-refresh`,
        {
          method: "POST",
          headers: {
            Authorization: `Bearer ${accessToken}`,
            "Content-Type": "application/json",
          },
          body: JSON.stringify({
            refresh_token: refreshToken,
          }),
        },
      );

      const response = await Promise.race([
        fetchPromise,
        new Promise<Response>((_, reject) =>
          setTimeout(
            () =>
              reject(
                new Error(
                  `Token validate-and-refresh timeout after ${timeoutMs}ms`,
                ),
              ),
            timeoutMs,
          ),
        ),
      ]);

      if (!response.ok) {
        this.log(
          `Validate-and-refresh request failed: ${response.status} ${response.statusText}`,
        );
        return null;
      }

      const result = (await response.json()) as ValidateRefreshResponse;
      this.log(`Token validation status: ${result.status}`);

      if (result.status === "refreshed") {
        await this.adoptRefreshedTokens(result);
      }

      if (result.status === "invalid" && result.reason) {
        this.log(`Token invalid reason: ${result.reason}`);
      }

      return result;
    } catch (error) {
      this.log(`Validate-and-refresh request failed: ${error}`);
      return null;
    }
  }

  async redirectToAuthorization(authorizationUrl: URL): Promise<void> {
    // Non-interactive guard. When browser opening is disabled (the --no-browser
    // flag, PIERRE_DISABLE_BROWSER, or CI) an interactive OAuth flow can never
    // complete — a human has to approve in the browser. Refuse BEFORE starting a
    // callback server or awaiting the (never-arriving) callback. Suppressing only
    // the popup is not enough: the callback server would still bind a port and the
    // bridge would hang forever on it, orphaning the process (leaked port) and — on
    // hosts where the popup isn't suppressed — accreting browser tabs until Chrome
    // OOMs. Real MCP hosts (Claude Desktop etc.) set none of these, so interactive
    // OAuth still works there.
    const envDisabled =
      process.env.PIERRE_DISABLE_BROWSER === "true" ||
      process.env.CI === "true" ||
      process.env.GITHUB_ACTIONS === "true";
    if (this.config?.disableBrowser || envDisabled) {
      this.log(
        "Browser disabled - refusing interactive OAuth authorization (non-interactive mode)",
      );
      this.log(`Authorization URL (not opened): ${authorizationUrl.toString()}`);
      throw new Error(
        "Interactive OAuth authorization required but browser opening is disabled (non-interactive mode)",
      );
    }

    this.log(`Starting OAuth 2.0 authorization flow`);

    // Start callback server to receive authorization response
    await this.startCallbackServer();

    this.log(`Opening browser for authorization`);

    // Open the authorization URL in the user's default browser with focus
    openUrlInBrowserWithFocus(authorizationUrl.toString(), {
      disableBrowser: this.config?.disableBrowser,
      log: (message) => this.log(message),
    });

    this.log(`If browser doesn't open automatically, visit:`);
    this.log(`${authorizationUrl.toString()}`);
    this.log(`Waiting for authorization completion`);

    await this.completeAuthorization();
  }

  /**
   * Waits for the browser callback, exchanges the code, and tears the flow down.
   * Teardown is unconditional: an abandoned authorization leaves no listener on the
   * callback port and no code verifier or state in memory for a later flow to reuse.
   */
  private async completeAuthorization(): Promise<void> {
    try {
      const authResult = await this.awaitAuthorizationCallback();
      this.log(`Authorization callback completed, exchanging code for tokens`);

      // Exchange authorization code for JWT token
      await this.exchangeCodeForTokens(authResult.code, authResult.state);
    } finally {
      this.teardownAuthorizationFlow();
    }
  }

  private async awaitAuthorizationCallback(): Promise<{
    code: string;
    state: string;
  }> {
    const pending = this.authorizationPending;
    if (!pending) {
      throw new PierreError(
        PierreErrorCode.AUTH_ERROR,
        "Authorization wait was not armed - callback server was not started",
      );
    }

    const timeoutMs =
      this.config.authorizationTimeoutMs || DEFAULT_AUTHORIZATION_TIMEOUT_MS;

    return new Promise<{ code: string; state: string }>((resolve, reject) => {
      const timer = setTimeout(() => {
        reject(
          new PierreError(
            PierreErrorCode.TIMEOUT_ERROR,
            `OAuth authorization was not completed within ${Math.round(timeoutMs / 1000)}s. The browser approval was never received - run the connect tool again to start a new authorization.`,
          ),
        );
      }, timeoutMs);

      pending.then(
        (result) => {
          clearTimeout(timer);
          resolve(result);
        },
        (error) => {
          clearTimeout(timer);
          reject(error);
        },
      );
    });
  }

  /**
   * Ends the authorization flow: closes the callback listener (including keep-alive
   * sockets, which would otherwise hold the port), and drops the per-flow secrets.
   * The next flow starts a fresh listener with a fresh state, verifier and callback token.
   */
  private teardownAuthorizationFlow(): void {
    const server = this.callbackServer;

    this.callbackServer = undefined;
    this.callbackServerReady = undefined;
    this.callbackBindError = undefined;
    this.callbackPort = 0;
    this.callbackSessionToken = undefined;
    this.authorizationPending = undefined;
    this.codeVerifierValue = undefined;
    this.stateValue = undefined;

    if (server) {
      server.removeAllListeners("request");
      delete server._authResolve;
      delete server._authReject;
      server.close(() => undefined);
      server.closeAllConnections?.();
    }

    this.log(
      "Authorization flow closed: callback listener stopped, state and code verifier cleared",
    );
  }

  private async exchangeCodeForTokens(
    authorizationCode: string,
    state: string,
  ): Promise<void> {
    // Validate OAuth state parameter to prevent CSRF attacks
    if (!this.stateValue || state !== this.stateValue) {
      this.log(
        `OAuth state mismatch: expected=${this.stateValue ? "[set]" : "[unset]"}, received=${state ? "[set]" : "[unset]"}`,
      );
      throw new PierreError(
        PierreErrorCode.AUTH_ERROR,
        "OAuth state parameter mismatch - possible CSRF attack. Please try connecting again.",
      );
    }

    if (!this.clientInfo) {
      throw new PierreError(PierreErrorCode.AUTH_ERROR, "Client information not available for token exchange");
    }

    if (!this.clientInfo.client_secret) {
      throw new PierreError(PierreErrorCode.AUTH_ERROR, "Client secret not available for token exchange");
    }

    if (!this.codeVerifierValue) {
      throw new PierreError(PierreErrorCode.AUTH_ERROR, "Code verifier not available for token exchange");
    }

    const tokenEndpoint = `${this.serverUrl}/oauth2/token`;
    const tokenRequestBody = new URLSearchParams({
      grant_type: "authorization_code",
      code: authorizationCode,
      redirect_uri: this.redirectUrl,
      client_id: this.clientInfo.client_id,
      client_secret: this.clientInfo.client_secret,
      code_verifier: this.codeVerifierValue,
    });

    this.log(`Requesting JWT token from ${tokenEndpoint}`);

    try {
      // Use native fetch (Node 24+)
      const response = await fetch(tokenEndpoint, {
        method: "POST",
        headers: {
          "Content-Type": "application/x-www-form-urlencoded",
          Accept: "application/json",
        },
        body: tokenRequestBody.toString(),
      });

      if (!response.ok) {
        const errorText = await response.text();
        throw new PierreError(
          PierreErrorCode.AUTH_ERROR,
          `Token exchange failed: ${response.status} ${response.statusText}: ${errorText}`,
        );
      }

      const tokenResponse = (await response.json()) as OAuth2TokenResponse;
      this.log(
        `Successfully received JWT token, expires_in=${tokenResponse.expires_in}`,
      );

      // Convert to MCP SDK OAuthTokens format and save
      const oauthTokens: OAuthTokens = {
        access_token: tokenResponse.access_token,
        token_type: tokenResponse.token_type,
        expires_in: tokenResponse.expires_in,
        refresh_token: tokenResponse.refresh_token,
        scope: tokenResponse.scope,
      };

      await this.saveTokens(oauthTokens);
    } catch (error) {
      this.log(`Token exchange failed: ${error}`);
      throw error;
    }
  }

  async saveCodeVerifier(codeVerifier: string): Promise<void> {
    this.codeVerifierValue = codeVerifier;
  }

  async codeVerifier(): Promise<string> {
    if (!this.codeVerifierValue) {
      throw new PierreError(
        PierreErrorCode.AUTH_ERROR,
        "Code verifier not found - authorization flow not started",
      );
    }
    return this.codeVerifierValue;
  }

  async invalidateCredentials(
    scope: "all" | "client" | "tokens" | "verifier",
  ): Promise<void> {
    switch (scope) {
      case "all":
        this.clientInfo = undefined;
        this.savedTokens = undefined;
        this.codeVerifierValue = undefined;
        this.stateValue = undefined;
        // Clear all stored tokens
        this.allStoredTokens = {};
        await this.saveStoredTokens();
        break;
      case "client":
        this.clientInfo = undefined;
        delete this.allStoredTokens.client_info;
        await this.saveStoredTokens();
        break;
      case "tokens":
        this.savedTokens = undefined;
        // Note: We intentionally do NOT clear persistent storage here
        // This allows token reload from storage if the tokens are still valid
        // Only clear persistent storage if tokens are truly expired
        break;
      case "verifier":
        this.codeVerifierValue = undefined;
        break;
    }
    this.log(`Invalidated credentials: ${scope}`);
  }

  async clearProviderTokens(): Promise<void> {
    this.allStoredTokens.providers = {};
    await this.saveStoredTokens();
    this.log(`Cleared all provider tokens from client storage`);
  }

  getTokenStatus(): { pierre: boolean; providers: Record<string, boolean> } {
    const status = {
      pierre: false,
      providers: {} as Record<string, boolean>,
    };

    // Check Dravr token
    status.pierre = !!this.savedTokens;

    // Check provider tokens from client-side storage
    if (this.allStoredTokens.providers) {
      for (const [provider, tokenData] of Object.entries(
        this.allStoredTokens.providers,
      )) {
        // Check if token exists and is not expired
        const isValid =
          tokenData &&
          (!tokenData.expires_at || Date.now() < tokenData.expires_at);
        status.providers[provider] = !!isValid;
      }
    }

    return status;
  }

  public generateRandomString(length: number): string {
    const chars =
      "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    const bytes = randomBytes(length);
    let result = "";
    for (let i = 0; i < length; i++) {
      result += chars.charAt(bytes[i] % chars.length);
    }
    return result;
  }

  public async generateCodeChallenge(codeVerifier: string): Promise<string> {
    // Generate SHA-256 hash of the code verifier
    const hash = createHash("sha256").update(codeVerifier).digest();

    // Base64 URL encode the hash
    return hash
      .toString("base64")
      .replace(/\+/g, "-")
      .replace(/\//g, "_")
      .replace(/=/g, "");
  }

  private startCallbackServerSync(): void {
    // Binds the configured callback port and only that port. The redirect URI published
    // to the authorization server names this port, and a provider delivers the
    // authorization code to whichever local process holds it - so moving to a different
    // port after publishing would hand the code to the process squatting the published
    // one. A port that cannot be bound is therefore a hard failure, surfaced through
    // ensureCallbackServerBound() before the authorization request is ever sent.
    const http = require("http");

    if (this.callbackServer) {
      return; // Already started
    }

    const port = this.config.callbackPort || DEFAULT_CALLBACK_PORT;

    this.callbackBindError = undefined;
    this.callbackPort = port;
    // Per-flow secret proving a provider token callback belongs to this flow. Generated
    // with the server so it is available to whoever kicks the flow off, not only once
    // the asynchronous listen() completes.
    this.callbackSessionToken = randomBytes(32).toString("hex");

    const server = http.createServer();
    this.callbackServer = server;

    this.callbackServerReady = new Promise<number>((resolve, reject) => {
      const onListening = (): void => {
        server.removeListener("error", onError);
        this.callbackPort = server.address().port;
        this.log(
          `Callback server listening on http://localhost:${this.callbackPort}/oauth/callback`,
        );
        this.setupCallbackHandler();
        resolve(this.callbackPort);
      };

      const onError = (error: any): void => {
        server.removeListener("listening", onListening);
        const message =
          error.code === "EADDRINUSE"
            ? `OAuth callback port ${port} is already in use. The redirect URI http://localhost:${port}/oauth/callback is published to the authorization server, so the authorization code would be delivered to the process holding that port instead of this client. Free the port (lsof -ti:${port} | xargs kill) or start the client with --callback-port <free port>, then retry.`
            : `OAuth callback server failed to bind port ${port}: ${error.message}`;
        this.callbackBindError = new PierreError(
          PierreErrorCode.CONFIG_ERROR,
          message,
        );
        this.callbackPort = 0;
        this.callbackServer = undefined;
        this.log(message);
        server.close(() => undefined);
        reject(this.callbackBindError);
      };

      server.once("listening", onListening);
      server.once("error", onError);
      server.listen(port, "localhost");
    });

    // ensureCallbackServerBound() is the awaiter. Attach a sink so a bind failure that
    // nobody has awaited yet does not surface as an unhandled rejection.
    this.callbackServerReady.catch(() => undefined);
  }

  /**
   * Resolves with the port the callback listener actually holds, rejecting with an
   * actionable error if it could not be bound. Callers must await this before sending
   * the redirect URI anywhere, so the published port is always a port this process owns.
   */
  public async ensureCallbackServerBound(): Promise<number> {
    if (!this.callbackServer && this.callbackPort === 0) {
      this.startCallbackServerSync();
    }
    if (this.callbackBindError) {
      throw this.callbackBindError;
    }
    if (!this.callbackServerReady) {
      throw new PierreError(
        PierreErrorCode.CONFIG_ERROR,
        "Callback server was never started - no port available",
      );
    }
    return this.callbackServerReady;
  }

  /**
   * Per-flow secret a provider token callback must present on the local endpoint.
   * Available as soon as the callback server is created, so the flow initiator can hand
   * it to the party that will post the provider tokens back.
   */
  public get callbackAuthToken(): string | undefined {
    return this.callbackSessionToken;
  }

  private setupCallbackHandler(): void {
    if (!this.callbackServer) return;

    this.callbackServer.removeAllListeners("request");
    this.callbackServer.on("request", async (req: any, res: any) => {
      try {
        if (!req.url) {
          res.writeHead(400, { "Content-Type": "text/plain" });
          res.end("Bad Request: No URL provided");
          return;
        }

        const url = require("url");
        const parsedUrl = url.parse(req.url, true);

        if (parsedUrl.pathname === "/oauth/callback") {
          const query = parsedUrl.query;

          if (query.error) {
            this.log(`Authorization failed: ${query.error}`);
            res.writeHead(400, { "Content-Type": "text/html" });
            res.end(
              this.renderErrorTemplate(
                "MCP host OAuth",
                `${query.error}`,
                `${query.error_description || "Please try connecting again."}`,
              ),
            );
          } else if (query.code && query.state) {
            this.log(`Authorization successful, received code`);
            res.writeHead(200, { "Content-Type": "text/html" });
            res.end(this.renderSuccessTemplate("MCP host OAuth"));

            // Resolve the authorization promise if it exists
            const authResolve = (this.callbackServer as any)._authResolve;
            if (authResolve) {
              authResolve({ code: query.code, state: query.state });
            }

            // Authorization result is handled via promise resolution in redirectToAuthorization()
          } else {
            this.log(`Invalid callback parameters`);
            res.writeHead(400, { "Content-Type": "text/html" });
            res.end(
              this.renderErrorTemplate(
                "MCP host OAuth",
                "Invalid Request",
                "Missing required authorization parameters. Please try connecting again.",
              ),
            );
          }
        } else if (parsedUrl.pathname?.startsWith("/oauth/provider-callback")) {
          this.handleProviderTokenCallback(req, res, parsedUrl);
          return; // Response is written by the handler, after the body is read
        } else {
          res.writeHead(404, { "Content-Type": "text/plain" });
          res.end("Not Found");
        }
      } catch (error) {
        this.log(`Callback server error: ${error}`);
        res.writeHead(500, { "Content-Type": "text/plain" });
        res.end("Internal Server Error");
      }
    });
  }

  /**
   * Provider token callback endpoint. Whatever it accepts is written into the user's
   * credential store, and the listener is reachable by every process on the machine and
   * by any page the user's browser loads - so a request is only accepted when it proves
   * it belongs to the flow this process started: exact method and path, no browser origin,
   * loopback host and peer, and the per-flow callback token. Everything else is refused
   * before the body is parsed and nothing is stored.
   */
  private handleProviderTokenCallback(req: any, res: any, parsedUrl: any): void {
    const reject = (
      status: number,
      message: string,
      logLine: string,
    ): void => {
      this.log(logLine);
      res.writeHead(status, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ success: false, message }));
    };

    if (req.method !== "POST") {
      reject(
        405,
        "Method not allowed - provider token callbacks are POST only",
        `Rejected provider callback: method ${req.method}`,
      );
      return;
    }

    // Exact path match. Prefix matching accepted trailing segments, which let a caller
    // dress an arbitrary request up as a callback for a provider it named itself.
    const pathMatch = PROVIDER_CALLBACK_PATH.exec(parsedUrl.pathname);
    if (!pathMatch) {
      reject(
        404,
        "Unknown callback path",
        `Rejected provider callback: unsupported path ${parsedUrl.pathname}`,
      );
      return;
    }
    const provider = pathMatch[1];

    // Host header and source IP must both be loopback: the header alone can be spoofed,
    // and the peer address alone does not stop a DNS-rebound page from reaching us.
    const host = req.headers.host;
    if (
      !host ||
      !(host.startsWith("localhost") || host.startsWith("127.0.0.1"))
    ) {
      reject(
        403,
        "Invalid host - only localhost allowed",
        `Rejected POST callback for ${provider}: Invalid host ${host}`,
      );
      return;
    }

    const remoteAddress = req.socket?.remoteAddress;
    if (
      remoteAddress &&
      remoteAddress !== "127.0.0.1" &&
      remoteAddress !== "::1" &&
      remoteAddress !== "::ffff:127.0.0.1"
    ) {
      reject(
        403,
        "Invalid source - only localhost allowed",
        `Rejected POST callback for ${provider}: Non-localhost source IP ${remoteAddress}`,
      );
      return;
    }

    // Origin or Referer means a web page issued this request. The OAuth notification is
    // a server-to-client call and carries neither, so their presence identifies the one
    // caller class that must never be able to plant credentials.
    const origin = req.headers.origin;
    const referer = req.headers.referer;
    if (origin || referer) {
      reject(
        403,
        "Browser-originated requests are not accepted on this endpoint",
        `Rejected POST callback for ${provider}: browser origin=${origin ?? "none"} referer=${referer ?? "none"}`,
      );
      return;
    }

    const presentedToken = this.readCallbackToken(req, parsedUrl);
    if (
      !this.callbackSessionToken ||
      !presentedToken ||
      !constantTimeEquals(presentedToken, this.callbackSessionToken)
    ) {
      reject(
        403,
        "Invalid or missing callback authentication token",
        `Rejected POST callback for ${provider}: callback token ${presentedToken ? "mismatch" : "absent"}`,
      );
      return;
    }

    this.log(`Provider token callback for ${provider}`);

    let body = "";
    req.on("data", (chunk: any) => {
      body += chunk.toString();
    });

    req.on("end", async () => {
      try {
        const tokenData = JSON.parse(body);
        if (
          typeof tokenData?.access_token !== "string" ||
          tokenData.access_token.length === 0
        ) {
          reject(
            400,
            "Provider token payload has no access_token",
            `Rejected POST callback for ${provider}: payload has no access_token`,
          );
          return;
        }

        await this.saveProviderToken(provider, tokenData);

        // Resolve pending provider OAuth promise (allows handleConnectProvider to continue)
        this.resolveProviderOAuth(provider);

        // Notify PierreMcpClient about provider OAuth completion (for MCP notification)
        if (this.onProviderOAuthComplete) {
          try {
            await this.onProviderOAuthComplete(provider);
            this.log(`Notified MCP client about ${provider} OAuth completion`);
          } catch (notifyError) {
            this.log(
              `Failed to notify MCP client about ${provider} OAuth: ${notifyError}`,
            );
          }
        }

        res.writeHead(200, { "Content-Type": "application/json" });
        res.end(
          JSON.stringify({
            success: true,
            message: `${provider} token stored client-side`,
          }),
        );
      } catch (error) {
        this.log(`Failed to save ${provider} token: ${error}`);
        res.writeHead(400, { "Content-Type": "application/json" });
        res.end(
          JSON.stringify({
            success: false,
            message: "Failed to save provider token",
          }),
        );
      }
    });
  }

  private readCallbackToken(req: any, parsedUrl: any): string | undefined {
    const headerToken = req.headers["x-callback-token"];
    if (typeof headerToken === "string" && headerToken.length > 0) {
      return headerToken;
    }
    const queryToken = parsedUrl.query?.callback_token;
    if (typeof queryToken === "string" && queryToken.length > 0) {
      return queryToken;
    }
    return undefined;
  }

  private async startCallbackServer(): Promise<void> {
    // Bind before arming the wait: the redirect URI carried in the authorization request
    // names the callback port, so the port is owned by this process before that request
    // leaves. A bind failure throws here rather than degrading to another port.
    await this.ensureCallbackServerBound();

    this.authorizationPending = new Promise((resolve, reject) => {
      // Store resolve/reject for the callback handler to use
      (this.callbackServer as any)._authResolve = resolve;
      (this.callbackServer as any)._authReject = reject;
    });
  }

  private renderSuccessTemplate(provider: string): string {
    return OAUTH_SUCCESS_TEMPLATE.replace(/\{\{PROVIDER\}\}/g, this.escapeHtml(provider));
  }

  // Security: Escape HTML special characters to prevent XSS attacks
  // Query parameters like error and error_description come from external sources
  private escapeHtml(unsafe: string): string {
    return unsafe
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
  }

  private renderErrorTemplate(
    provider: string,
    error: string,
    description: string,
  ): string {
    // Security: Escape all user-provided values to prevent reflected XSS
    // The error and description values come from OAuth callback query parameters
    return OAUTH_ERROR_TEMPLATE.replace(/\{\{PROVIDER\}\}/g, this.escapeHtml(provider))
      .replace(/\{\{ERROR\}\}/g, this.escapeHtml(error))
      .replace(/\{\{DESCRIPTION\}\}/g, this.escapeHtml(description));
  }

  async validateAndCleanupCachedCredentials(): Promise<void> {
    const existingTokens = await this.tokens();
    const clientInfo = await this.clientInformation();

    if (!existingTokens && !clientInfo) {
      this.log("No cached credentials found - fresh start");
      return;
    }

    // Skip validation when using the JWT from PIERRE_JWT_TOKEN
    // These tokens are validated by the server on each request, not pre-validated
    if (this.config.mode === 'jwt') {
      this.log(
        "Skipping credential validation (using the JWT token from the environment)",
      );
      return;
    }

    const timeoutMs = this.config.tokenValidationTimeoutMs || 3000;
    this.log(
      `Validating cached credentials with server (timeout: ${timeoutMs}ms)...`,
    );

    // Try to validate by calling the token validation endpoint with configurable timeout
    try {
      const fetchPromise = fetch(`${this.serverUrl}/oauth2/token-validate`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          ...(existingTokens
            ? { Authorization: `Bearer ${existingTokens.access_token}` }
            : {}),
        },
        body: JSON.stringify({
          client_id: clientInfo?.client_id,
        }),
      });

      const response = await Promise.race([
        fetchPromise,
        new Promise<Response>((_, reject) =>
          setTimeout(
            () =>
              reject(
                new Error(`Token validation timeout after ${timeoutMs}ms`),
              ),
            timeoutMs,
          ),
        ),
      ]);

      const result: any = await response.json();

      if (result.valid === false) {
        this.log(
          `Cached credentials are invalid: ${result.error || "unknown error"}`,
        );
        this.log("Cleaning up invalid cached credentials...");

        // Clear invalid tokens from keychain
        if (existingTokens && this.secureStorage) {
          await this.secureStorage.clearTokens();
          this.log("Cleared invalid tokens from keychain");
        }

        // Clear invalid client info from secure storage
        if (clientInfo) {
          await this.clearClientInfo();
          this.log("Cleared invalid client registration from secure storage");
        }

        // Reset in-memory state
        this.savedTokens = undefined;
        this.clientInfo = undefined;
        this.allStoredTokens = {};
      } else {
        this.log("Cached credentials are valid");
      }
    } catch (error: any) {
      this.log(`Failed to validate credentials: ${error.message}`);
      this.log(
        "Will proceed with cached credentials and handle errors during connection",
      );
    }
  }
}
