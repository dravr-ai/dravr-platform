// ABOUTME: Astro middleware that guards /docs/* routes behind Supabase auth
// ABOUTME: Checks session cookie, verifies approval status, redirects unauthenticated requests

import { defineMiddleware } from 'astro:middleware';
import { createClient } from '@supabase/supabase-js';

const PUBLIC_DOCS_PATHS = ['/docs/login', '/docs/auth/callback'];

export const onRequest = defineMiddleware(async (context, next) => {
  const { pathname } = context.url;

  if (!pathname.startsWith('/docs/')) {
    return next();
  }

  if (PUBLIC_DOCS_PATHS.some((p) => pathname.startsWith(p))) {
    return next();
  }

  // Build Supabase client that reads cookies from the request
  const supabase = createClient(
    import.meta.env.SUPABASE_URL,
    import.meta.env.SUPABASE_SERVICE_ROLE_KEY,
    { auth: { autoRefreshToken: false, persistSession: false } },
  );

  // Extract Supabase session token from cookie
  const cookieHeader = context.request.headers.get('cookie') ?? '';
  const tokenMatch = cookieHeader.match(/sb-access-token=([^;]+)/);
  const accessToken = tokenMatch ? decodeURIComponent(tokenMatch[1]) : null;

  if (!accessToken) {
    return context.redirect('/docs/login');
  }

  const { data: { user }, error } = await supabase.auth.getUser(accessToken);

  if (error || !user) {
    return context.redirect('/docs/login');
  }

  // Verify the user is on the approved waitlist
  const { data: waitlistEntry } = await supabase
    .from('waitlist')
    .select('status')
    .eq('email', user.email ?? '')
    .single();

  if (!waitlistEntry || waitlistEntry.status !== 'approved') {
    return context.redirect('/docs/login?reason=not-approved');
  }

  context.locals.user = user;
  return next();
});
