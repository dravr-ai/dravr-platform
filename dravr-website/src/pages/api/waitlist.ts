// ABOUTME: API route for waitlist signups — inserts email into Supabase waitlist table
// ABOUTME: Verifies Cloudflare Turnstile challenge, validates email, uses service-role client

import type { APIRoute } from 'astro';
import { createAdminClient } from '../../lib/supabase';

const EMAIL_REGEX = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
const TURNSTILE_VERIFY_URL = 'https://challenges.cloudflare.com/turnstile/v0/siteverify';

interface TurnstileResponse {
  success: boolean;
  'error-codes'?: string[];
}

async function verifyTurnstile(token: string, remoteip: string | null): Promise<boolean> {
  const body = new URLSearchParams({
    secret: import.meta.env.TURNSTILE_SECRET_KEY,
    response: token,
  });
  if (remoteip) {
    body.set('remoteip', remoteip);
  }

  const response = await fetch(TURNSTILE_VERIFY_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body,
  });

  const result = (await response.json()) as TurnstileResponse;
  return result.success === true;
}

export const POST: APIRoute = async ({ request }) => {
  let email: string;
  let turnstileToken: string;

  try {
    const body = (await request.json()) as { email?: unknown; turnstileToken?: unknown };
    email = typeof body.email === 'string' ? body.email.trim().toLowerCase() : '';
    turnstileToken = typeof body.turnstileToken === 'string' ? body.turnstileToken : '';
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

  if (!turnstileToken) {
    return new Response(JSON.stringify({ error: 'Challenge required.' }), {
      status: 400,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  const remoteip = request.headers.get('CF-Connecting-IP');
  const challengeOk = await verifyTurnstile(turnstileToken, remoteip);
  if (!challengeOk) {
    return new Response(JSON.stringify({ error: 'Challenge verification failed.' }), {
      status: 403,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  const supabase = createAdminClient();

  const { error } = await supabase
    .from('waitlist')
    .insert({ email, source: 'website' });

  // Duplicate signups (unique-constraint violation, code 23505) are treated as
  // successful to avoid leaking which emails are already on the waitlist.
  if (error && error.code !== '23505') {
    console.error('waitlist insert failed', { code: error.code });
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
