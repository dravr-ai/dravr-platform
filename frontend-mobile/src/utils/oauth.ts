// ABOUTME: OAuth utility functions for mobile app
// ABOUTME: Handles OAuth callback URL generation for development and production

// The app scheme declared in app.config.js. The server's redirect allowlist
// (pierre-services/src/oauth_redirects.rs, APP_SCHEMES) must admit the same
// scheme, or the post-OAuth hop back into the app is rejected.
const APP_SCHEME = 'dravr';

/**
 * Creates the OAuth callback URL for the mobile app.
 *
 * In Expo Go development, Linking.createURL() returns exp://... URLs which
 * don't work on physical devices. This function returns the custom scheme
 * URL directly (dravr://oauth-callback) which works in both development
 * and production.
 *
 * For WebBrowser.openAuthSessionAsync to work, the scheme must be registered:
 * - In Expo Go: The dravr:// scheme is handled via the app.config.js scheme
 * - In standalone builds: Universal links or app links handle the redirect
 */
export function getOAuthCallbackUrl(): string {
  // Always use the custom scheme for OAuth callbacks
  // This ensures consistent behavior across dev and prod
  return `${APP_SCHEME}://oauth-callback`;
}
