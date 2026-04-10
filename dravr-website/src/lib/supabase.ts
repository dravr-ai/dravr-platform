// ABOUTME: Supabase client factories for the Dravr website
// ABOUTME: Browser (publishable), Astro server (publishable + cookie session), admin (secret key)

import { createClient } from '@supabase/supabase-js';
import {
  createBrowserClient as createBrowserClientSsr,
  createServerClient as createServerClientSsr,
  parseCookieHeader,
} from '@supabase/ssr';
import type { AstroCookies } from 'astro';

// Browser-side: publishable key, reads/writes session via document.cookie.
// Used from `<script>` tags in login/callback pages.
export function createBrowserClient() {
  return createBrowserClientSsr(
    import.meta.env.PUBLIC_SUPABASE_URL,
    import.meta.env.PUBLIC_SUPABASE_PUBLISHABLE_KEY,
  );
}

// Server-side: publishable key, reads/writes session via Astro cookies.
// Reads the Supabase session from the request `Cookie` header and writes
// refreshed tokens back through `AstroCookies.set()`. Automatically rotates
// refresh tokens — callers should always use `supabase.auth.getUser()` to
// ensure the refresh runs before accessing the user.
export function createAstroServerClient(request: Request, cookies: AstroCookies) {
  return createServerClientSsr(
    import.meta.env.PUBLIC_SUPABASE_URL,
    import.meta.env.PUBLIC_SUPABASE_PUBLISHABLE_KEY,
    {
      cookies: {
        getAll() {
          return parseCookieHeader(request.headers.get('cookie') ?? '').map(
            ({ name, value }) => ({ name, value: value ?? '' }),
          );
        },
        setAll(cookiesToSet) {
          for (const { name, value, options } of cookiesToSet) {
            cookies.set(name, value, {
              ...options,
              httpOnly: true,
              secure: true,
              sameSite: 'strict',
              path: '/',
            });
          }
        },
      },
    },
  );
}

// Admin client: secret key, no cookie handling.
// Used by the waitlist API to insert rows bypassing RLS, and by middleware
// to check waitlist approval status for the authenticated user.
export function createAdminClient() {
  return createClient(
    import.meta.env.PUBLIC_SUPABASE_URL,
    import.meta.env.SUPABASE_SECRET_KEY,
    {
      auth: {
        autoRefreshToken: false,
        persistSession: false,
      },
    },
  );
}
