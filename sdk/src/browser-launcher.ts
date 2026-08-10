// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Single launcher used by the SDK to open an OAuth URL in the user's browser
// ABOUTME: Accepts only http/https and passes the URL as an argv element, never through a shell

import { execFile } from "child_process";

/** Sink for launcher diagnostics, so output lands in the calling component's log stream. */
export type LaunchLogger = (message: string) => void;

export interface OpenUrlOptions {
  /** The caller's --no-browser configuration flag. */
  disableBrowser?: boolean;
  /** Where the launcher reports refusals and failures. */
  log: LaunchLogger;
}

/**
 * Browsers activated on macOS after the URL is handed to `open`, in preference order.
 * These are fixed literals: the activation step never interpolates the URL or any
 * other caller-supplied value into the AppleScript it runs.
 */
const MACOS_ACTIVATION_APPS = [
  "Google Chrome",
  "Safari",
  "Firefox",
  "Brave Browser",
] as const;

/** Delay before activating, so `open` has handed the URL to the browser first. */
const MACOS_ACTIVATION_DELAY_MS = 500;

/**
 * Activate the first browser in MACOS_ACTIVATION_APPS that responds, starting at
 * `index`. Each candidate is tried only after the previous one fails, and running
 * out of candidates is non-fatal — the URL is already open, only focus is missing.
 */
function activateMacOsBrowser(index: number, log: LaunchLogger): void {
  if (index >= MACOS_ACTIVATION_APPS.length) {
    log("Could not activate browser (non-fatal)");
    return;
  }

  const app = MACOS_ACTIVATION_APPS[index];
  execFile(
    "osascript",
    ["-e", `tell application "${app}" to activate`],
    (error) => {
      if (error) {
        activateMacOsBrowser(index + 1, log);
      }
    },
  );
}

/**
 * Open an authorization URL in the user's default browser and bring it to the front.
 *
 * The URL is attacker-influenced input: the MCP SDK calls the OAuth provider's
 * redirectToAuthorization with an endpoint taken from the server's discovery
 * document (or a WWW-Authenticate header), so the host need not be Dravr. It is
 * therefore never interpolated into a command string — the launcher rejects any
 * scheme other than http/https and hands the URL to the platform opener as a single
 * argv element, so shell metacharacters (which survive URL serialization verbatim in
 * the fragment) reach the browser as text instead of the shell as code.
 */
export function openUrlInBrowserWithFocus(
  url: string,
  options: OpenUrlOptions,
): void {
  const { log } = options;

  // Check if browser opening is disabled (testing mode). The env kill switch
  // (PIERRE_DISABLE_BROWSER / CI / GITHUB_ACTIONS) suppresses the popup even
  // when the config flag wasn't threaded through, so non-interactive runs
  // never open (and never OOM) a browser tab. Real MCP hosts set none of these.
  const envDisabled =
    process.env.PIERRE_DISABLE_BROWSER === "true" ||
    process.env.CI === "true" ||
    process.env.GITHUB_ACTIONS === "true";
  if (options.disableBrowser || envDisabled) {
    log("Browser opening disabled - OAuth URL available at callback server");
    log(`OAuth URL: ${url}`);
    return;
  }

  let parsedUrl: URL;
  try {
    parsedUrl = new URL(url);
  } catch {
    log("Invalid URL format, refusing to open");
    return;
  }

  if (parsedUrl.protocol !== "http:" && parsedUrl.protocol !== "https:") {
    log(`Refusing to open non-HTTP URL: ${parsedUrl.protocol}`);
    return;
  }

  const platform = process.platform;

  if (platform === "darwin") {
    // macOS: open the URL, then explicitly activate a browser after a brief delay
    execFile("open", [parsedUrl.href], (error) => {
      if (error) {
        log(`Failed to open browser: ${error.message}`);
        return;
      }

      setTimeout(
        () => activateMacOsBrowser(0, log),
        MACOS_ACTIVATION_DELAY_MS,
      );
    });
  } else if (platform === "win32") {
    // Windows: `start` is a cmd.exe builtin rather than an executable, and driving it
    // means running cmd.exe, which re-parses its arguments — an unquoted `&` inside a
    // URL would start a second command there. rundll32 invokes the same ShellExecute
    // the shell would, takes the URL as an argument, and has no command syntax of its
    // own, so the default browser opens with nothing left to interpret.
    execFile(
      "rundll32.exe",
      ["url.dll,FileProtocolHandler", parsedUrl.href],
      (error) => {
        if (error) {
          log(`Failed to open browser: ${error.message}`);
        }
      },
    );
  } else {
    // Linux and other Unix desktops: xdg-open picks the registered http/https handler
    execFile("xdg-open", [parsedUrl.href], (error) => {
      if (error) {
        log(`Failed to open browser: ${error.message}`);
      }
    });
  }
}
