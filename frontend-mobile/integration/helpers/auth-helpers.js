// ABOUTME: Authentication helper functions for mobile integration tests.
// ABOUTME: Provides real sign-in flows (hosted login page, authorization code + PKCE) against the actual backend server.

const crypto = require('crypto');
const { getBackendUrl } = require('./server-manager');
const { createTestAdminUser } = require('./db-setup');
const { testUsers } = require('../fixtures/test-data');

/** The app's OAuth client and the return address every deployment accepts for it. */
const MOBILE_CLIENT_ID = 'dravr-mobile';
const MOBILE_REDIRECT_URI = 'dravr://auth/callback';

/** Unpadded base64url (RFC 4648 §5) of a buffer. */
function base64Url(buffer) {
  return buffer.toString('base64').replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/** The `name=value` pairs of a response's Set-Cookie headers, as one Cookie header. */
function cookieHeader(response) {
  return response.headers
    .getSetCookie()
    .map((cookie) => cookie.split(';')[0])
    .join('; ');
}

/**
 * Perform a real sign-in the way the app does (carnet#787): the server's hosted
 * login page, the authorization code it redirects with, and the PKCE verifier
 * that redeems it. The password grant is gone from /oauth/token.
 *
 *   1. POST /oauth2/login      credentials + the authorization request
 *                              → 302 to /oauth2/authorize, with the session cookie
 *   2. GET  /oauth2/authorize  with that cookie
 *                              → 302 to dravr://auth/callback?code=…&state=…
 *                              (dravr-mobile skips the consent screen)
 *   3. POST /oauth/token       grant_type=authorization_code + code_verifier
 *
 * This makes actual API calls to the backend server.
 *
 * @param {string} email - User email
 * @param {string} password - User password
 * @returns {Promise<{success: boolean, accessToken?: string, refreshToken?: string, user?: object, error?: string}>}
 */
async function loginWithCredentials(email, password) {
  try {
    const backendUrl = getBackendUrl();
    console.log(`[Auth] Attempting login for ${email}`);

    const codeVerifier = base64Url(crypto.randomBytes(32));
    const codeChallenge = base64Url(crypto.createHash('sha256').update(codeVerifier).digest());
    const state = base64Url(crypto.randomBytes(16));

    // Step 1: the hosted login form. A refused password answers the form's own
    // error page (no redirect); a signed-in one redirects to the authorization
    // request with the authorization server's session cookie.
    const loginResponse = await fetch(`${backendUrl}/oauth2/login`, {
      method: 'POST',
      redirect: 'manual',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({
        client_id: MOBILE_CLIENT_ID,
        redirect_uri: MOBILE_REDIRECT_URI,
        response_type: 'code',
        state,
        scope: '',
        code_challenge: codeChallenge,
        code_challenge_method: 'S256',
        resource: '',
        email,
        password,
      }).toString(),
    });
    const authorizeLocation = loginResponse.headers.get('location');
    if (loginResponse.status !== 302 && loginResponse.status !== 303) {
      console.log(`[Auth] Login failed: hosted login page answered HTTP ${loginResponse.status}`);
      return {
        success: false,
        error:
          loginResponse.status === 429
            ? 'Too many sign-in attempts'
            : `Sign-in refused by the hosted login page (HTTP ${loginResponse.status})`,
      };
    }
    if (!authorizeLocation) {
      return { success: false, error: 'Hosted login page redirected nowhere' };
    }

    // Step 2: the authorization request, signed in by the session cookie.
    const authorizeResponse = await fetch(new URL(authorizeLocation, backendUrl), {
      redirect: 'manual',
      headers: { Cookie: cookieHeader(loginResponse) },
    });
    const callbackLocation = authorizeResponse.headers.get('location') ?? '';
    if (!callbackLocation.startsWith(`${MOBILE_REDIRECT_URI}?`)) {
      return {
        success: false,
        error: `Authorization answered HTTP ${authorizeResponse.status} without returning to ${MOBILE_REDIRECT_URI}`,
      };
    }
    const callback = new URLSearchParams(callbackLocation.slice(MOBILE_REDIRECT_URI.length + 1));
    if (callback.get('error')) {
      return {
        success: false,
        error: callback.get('error_description') || callback.get('error'),
      };
    }
    if (callback.get('state') !== state) {
      return { success: false, error: 'Authorization returned a mismatched state' };
    }
    const code = callback.get('code');
    if (!code) {
      return { success: false, error: 'Authorization redirect carried no code' };
    }

    // Step 3: redeem the code with the PKCE verifier, as the app does — with
    // offline_access, so a refresh token comes back too.
    const response = await fetch(`${backendUrl}/oauth/token`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/x-www-form-urlencoded',
        Accept: 'application/json',
      },
      body: new URLSearchParams({
        grant_type: 'authorization_code',
        client_id: MOBILE_CLIENT_ID,
        code,
        redirect_uri: MOBILE_REDIRECT_URI,
        code_verifier: codeVerifier,
        scope: 'offline_access',
      }).toString(),
    });

    const data = await response.json();

    if (!response.ok) {
      console.log(`[Auth] Login failed: ${data.error || response.statusText}`);
      return {
        success: false,
        error: data.error_description || data.error || 'Login failed',
      };
    }

    console.log(`[Auth] Login successful for ${email}`);
    return {
      success: true,
      accessToken: data.access_token,
      refreshToken: data.refresh_token,
      user: data.user,
    };
  } catch (error) {
    console.log(`[Auth] Login error: ${error.message}`);
    return {
      success: false,
      error: `Network error: ${error.message}`,
    };
  }
}

/**
 * Create a test admin user in the database and then log in.
 * This is the primary way to set up an authenticated session for tests.
 *
 * @returns {Promise<{success: boolean, accessToken?: string, refreshToken?: string, user?: object, error?: string}>}
 */
async function createAndLoginAsAdmin() {
  const user = testUsers.admin;
  console.log(`[Auth] Creating admin user: ${user.email}`);

  const createResult = await createTestAdminUser(user);
  if (!createResult.success) {
    console.log(`[Auth] Failed to create admin user: ${createResult.error}`);
    return { success: false, error: createResult.error };
  }
  console.log(`[Auth] Admin user created, proceeding to login`);

  return loginWithCredentials(user.email, user.password);
}

/**
 * Create a test super admin user and log in.
 *
 * @returns {Promise<{success: boolean, accessToken?: string, refreshToken?: string, user?: object, error?: string}>}
 */
async function createAndLoginAsSuperAdmin() {
  const user = testUsers.superAdmin;

  const createResult = await createTestAdminUser(user);
  if (!createResult.success) {
    return { success: false, error: createResult.error };
  }

  return loginWithCredentials(user.email, user.password);
}

/**
 * Create a custom test user and log in.
 *
 * @param {{email: string, password: string, role?: string}} user - User to create
 * @returns {Promise<{success: boolean, accessToken?: string, refreshToken?: string, user?: object, error?: string}>}
 */
async function createAndLoginTestUser(user) {
  const createResult = await createTestAdminUser(user);
  if (!createResult.success) {
    return { success: false, error: createResult.error };
  }

  return loginWithCredentials(user.email, user.password);
}

/**
 * Refresh an access token using a refresh token.
 *
 * @param {string} refreshToken - The refresh token
 * @returns {Promise<{success: boolean, accessToken?: string, refreshToken?: string, error?: string}>}
 */
async function refreshAccessToken(refreshToken) {
  try {
    const backendUrl = getBackendUrl();

    const response = await fetch(`${backendUrl}/oauth/token`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/x-www-form-urlencoded',
        Accept: 'application/json',
      },
      body: new URLSearchParams({
        grant_type: 'refresh_token',
        refresh_token: refreshToken,
      }).toString(),
    });

    const data = await response.json();

    if (!response.ok) {
      return {
        success: false,
        error: data.error_description || data.error || 'Token refresh failed',
      };
    }

    return {
      success: true,
      accessToken: data.access_token,
      refreshToken: data.refresh_token,
    };
  } catch (error) {
    return {
      success: false,
      error: `Network error: ${error.message}`,
    };
  }
}

/**
 * Make an authenticated API request.
 *
 * @param {string} endpoint - API endpoint (without base URL)
 * @param {string} accessToken - JWT access token
 * @param {object} options - Additional fetch options
 * @returns {Promise<{success: boolean, data?: any, status?: number, error?: string}>}
 */
async function authenticatedRequest(endpoint, accessToken, options = {}) {
  try {
    const backendUrl = getBackendUrl();
    const url = endpoint.startsWith('http')
      ? endpoint
      : `${backendUrl}${endpoint}`;

    const response = await fetch(url, {
      ...options,
      headers: {
        Accept: 'application/json',
        'Content-Type': 'application/json',
        Authorization: `Bearer ${accessToken}`,
        ...options.headers,
      },
    });

    const data = await response.json().catch(() => null);

    if (!response.ok) {
      return {
        success: false,
        status: response.status,
        error: data?.error || data?.message || response.statusText,
      };
    }

    return {
      success: true,
      status: response.status,
      data,
    };
  } catch (error) {
    return {
      success: false,
      error: `Network error: ${error.message}`,
    };
  }
}

/**
 * Check if a token is valid by making a request to a protected endpoint.
 *
 * @param {string} accessToken - JWT access token to verify
 * @returns {Promise<boolean>}
 */
async function isTokenValid(accessToken) {
  const result = await authenticatedRequest(
    '/api/chat/conversations',
    accessToken
  );
  return result.success || result.status !== 401;
}

module.exports = {
  loginWithCredentials,
  createAndLoginAsAdmin,
  createAndLoginAsSuperAdmin,
  createAndLoginTestUser,
  refreshAccessToken,
  authenticatedRequest,
  isTokenValid,
};
