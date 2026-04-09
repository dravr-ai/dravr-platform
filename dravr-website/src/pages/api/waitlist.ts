// ABOUTME: API route for waitlist signups — inserts email into Supabase waitlist table
// ABOUTME: Uses service role key (server-only); validates email format before insert

import type { APIRoute } from 'astro';
import { createServerClient } from '../../lib/supabase';

const EMAIL_REGEX = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

export const POST: APIRoute = async ({ request }) => {
  let email: string;

  try {
    const body = await request.json() as { email?: unknown };
    email = typeof body.email === 'string' ? body.email.trim().toLowerCase() : '';
  } catch {
    return new Response(JSON.stringify({ error: 'Invalid request body.' }), {
      status: 400,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  if (!email || !EMAIL_REGEX.test(email)) {
    return new Response(JSON.stringify({ error: 'Please enter a valid email address.' }), {
      status: 400,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  const supabase = createServerClient();

  const { error } = await supabase
    .from('waitlist')
    .insert({ email, source: 'website' });

  if (error) {
    // Unique constraint violation — already signed up
    if (error.code === '23505') {
      return new Response(JSON.stringify({ success: true, existing: true }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    return new Response(JSON.stringify({ error: 'Something went wrong. Please try again.' }), {
      status: 500,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  return new Response(JSON.stringify({ success: true }), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
};
