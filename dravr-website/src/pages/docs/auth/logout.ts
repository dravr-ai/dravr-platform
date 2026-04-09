// ABOUTME: Logout route — clears the session cookie and redirects to home
// ABOUTME: Client-side Supabase signOut is also triggered via script on the login page

import type { APIRoute } from 'astro';

export const GET: APIRoute = ({ cookies, redirect }) => {
  cookies.delete('sb-access-token', { path: '/' });
  return redirect('/');
};
