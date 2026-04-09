// ABOUTME: Supabase auth callback — exchanges magic link token for a session
// ABOUTME: Sets session cookie and redirects to /docs on success

import type { APIRoute } from 'astro';
import { createBrowserClient } from '../../../lib/supabase';

export const GET: APIRoute = async ({ url, redirect, cookies }) => {
  const code = url.searchParams.get('code');

  if (!code) {
    return redirect('/docs/login');
  }

  // createBrowserClient is used here because exchangeCodeForSession works client-side;
  // in SSR context we use it to exchange the code then persist the session via cookie.
  const supabase = createBrowserClient();
  const { data, error } = await supabase.auth.exchangeCodeForSession(code);

  const lang = url.searchParams.get('lang') === 'fr' ? 'fr' : 'en';

  if (error || !data.session) {
    return redirect(lang === 'fr' ? '/fr/docs/login' : '/docs/login');
  }

  // Persist the access token in a cookie for middleware to read
  cookies.set('sb-access-token', data.session.access_token, {
    httpOnly: true,
    secure: true,
    sameSite: 'lax',
    maxAge: data.session.expires_in,
    path: '/',
  });

  return redirect(lang === 'fr' ? '/fr/docs' : '/docs');
};
