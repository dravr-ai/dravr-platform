// ABOUTME: The trusted-domain gate every `url` reply action passes through before a client opens it
// ABOUTME: One allowlist for web and mobile — an action's value is server-declared, never blindly followed

/**
 * Registrable domains a reply action may send an athlete to.
 *
 * A `url` action's value reaches the client inside a model-adjacent reply, so
 * it is treated as an address the platform must vouch for rather than one it
 * merely received. The app's own origin is supplied per call instead of listed
 * here, because it differs between the browser and the packaged app.
 *
 * Deliberately spelled out rather than derived from `OAUTH_PROVIDERS` in
 * `@pierre/domain-utils`: that table decides what a *link label* says, so a row
 * added there for a nicer label would silently widen where a button can send
 * someone. Adding a host here is a security decision and reads like one.
 */
const TRUSTED_ACTION_DOMAINS: readonly string[] = [
  'strava.com',
  'garmin.com',
  'fitbit.com',
  'whoop.com',
  'tryterra.co',
  'dravr.ai',
];

/**
 * Does `hostname` sit on `domain`?
 *
 * Compares the registrable domain and its subdomains only. A prefix or
 * substring test would accept `strava.com.attacker.example`, which is the
 * whole reason an allowlist exists.
 */
function isOnDomain(hostname: string, domain: string): boolean {
  return hostname === domain || hostname.endsWith(`.${domain}`);
}

/**
 * Resolve a reply action's `url` value into an address safe to open.
 *
 * Returns the normalized absolute URL when the value is an `https:` (or
 * `http:`) address on a trusted domain, and `null` for everything else — a
 * relative path, a `javascript:`/`data:` scheme, an unparseable string, or a
 * host nobody vouched for. A `null` answer means the client opens nothing.
 *
 * @param raw the action's `value`, exactly as the server sent it.
 * @param appOrigins origins the running app itself serves — the browser's
 *   `location.origin` on web, the configured API base on mobile. The reconnect
 *   and short-link URLs the platform mints live here.
 */
export function trustedActionUrl(
  raw: string,
  appOrigins: readonly string[] = [],
): string | null {
  let parsed: URL;
  try {
    parsed = new URL(raw);
  } catch {
    return null;
  }

  if (parsed.protocol !== 'https:' && parsed.protocol !== 'http:') {
    return null;
  }

  for (const origin of appOrigins) {
    try {
      if (parsed.origin === new URL(origin).origin) return parsed.toString();
    } catch {
      // An origin the host could not supply is simply not a match.
      continue;
    }
  }

  const hostname = parsed.hostname.toLowerCase();
  const trusted = TRUSTED_ACTION_DOMAINS.some((domain) => isOnDomain(hostname, domain));
  return trusted ? parsed.toString() : null;
}
