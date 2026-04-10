// ABOUTME: Astro middleware that guards /docs/* and /fr/docs/* routes behind Supabase auth
// ABOUTME: Uses @supabase/ssr for session cookie handling + refresh; checks waitlist approval

import { defineMiddleware } from 'astro:middleware';
import { createAstroServerClient, createAdminClient } from './lib/supabase';

const PUBLIC_DOCS_PATHS = [
  '/docs/login',
  '/docs/auth/callback',
  '/docs/auth/logout',
  '/fr/docs/login',
];

function isProtectedDocsPath(pathname: string): boolean {
  return (
    pathname === '/docs' ||
    pathname === '/fr/docs' ||
    pathname.startsWith('/docs/') ||
    pathname.startsWith('/fr/docs/')
  );
}

function loginRedirectFor(pathname: string): string {
  return pathname.startsWith('/fr/') ? '/fr/docs/login' : '/docs/login';
}

export const onRequest = defineMiddleware(async (context, next) => {
  const { pathname } = context.url;

  if (!isProtectedDocsPath(pathname)) {
    return next();
  }

  if (PUBLIC_DOCS_PATHS.some((p) => pathname.startsWith(p))) {
    return next();
  }

  // Read the current session (transparently refreshes expired access tokens
  // and writes the new cookies back through context.cookies).
  const supabase = createAstroServerClient(context.request, context.cookies);
  const { data: { user }, error } = await supabase.auth.getUser();

  if (error || !user) {
    if (error) {
      console.warn('auth.getUser failed', { path: pathname, code: error.code });
    }
    return context.redirect(loginRedirectFor(pathname));
  }

  // Waitlist approval check uses the service-role client because the waitlist
  // table is locked down to service_role only (see RLS policies in
  // supabase/migrations/002_waitlist_policies.sql).
  const admin = createAdminClient();
  const { data: waitlistEntry } = await admin
    .from('waitlist')
    .select('status')
    .eq('email', user.email ?? '')
    .single();

  if (!waitlistEntry || waitlistEntry.status !== 'approved') {
    return context.redirect(`${loginRedirectFor(pathname)}?reason=not-approved`);
  }

  context.locals.user = user;
  return next();
});
