// ABOUTME: Logout route — revokes the Supabase session server-side and clears cookies
// ABOUTME: signOut() invalidates the refresh token at Supabase, not just the local cookie

import type { APIRoute } from 'astro';
import { createAstroServerClient } from '../../../lib/supabase';

export const GET: APIRoute = async ({ cookies, redirect, request }) => {
  const supabase = createAstroServerClient(request, cookies);
  await supabase.auth.signOut();
  return redirect('/');
};
