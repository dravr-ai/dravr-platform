// ABOUTME: Supabase auth callback — exchanges magic link token for a session
// ABOUTME: Persists the full session (access + refresh) via @supabase/ssr cookie helpers

import type { APIRoute } from 'astro';
import { createAstroServerClient } from '../../../lib/supabase';

export const GET: APIRoute = async ({ url, redirect, cookies, request }) => {
  const code = url.searchParams.get('code');
  const lang = url.searchParams.get('lang') === 'fr' ? 'fr' : 'en';
  const loginPath = lang === 'fr' ? '/fr/docs/login' : '/docs/login';
  const successPath = lang === 'fr' ? '/fr/docs' : '/docs';

  if (!code) {
    return redirect(loginPath);
  }

  // The ssr server client writes the refreshed session cookies back through
  // Astro's cookie API when exchangeCodeForSession succeeds — no manual
  // cookies.set() needed.
  const supabase = createAstroServerClient(request, cookies);
  const { error } = await supabase.auth.exchangeCodeForSession(code);

  if (error) {
    return redirect(loginPath);
  }

  return redirect(successPath);
};
