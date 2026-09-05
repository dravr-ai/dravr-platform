// ABOUTME: Landing page for /verify-email — renders the outcome the API redirected here with
// ABOUTME: Four outcomes, each with a different next action; never a dead end
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { authApi } from '../services/api';
import { Button, Card, Input } from './ui';
import { DravrLogo } from './DravrLogo';
import { useTranslation } from '@pierre/i18n';

/** The outcomes `GET /api/auth/verify-email` can redirect here with. */
export type VerifyEmailStatus = 'verified' | 'verified_pending' | 'invalid' | 'error';

/** Parse the `status` query parameter, defaulting to the failure case. */
export function parseVerifyStatus(search: string): VerifyEmailStatus {
  const value = new URLSearchParams(search).get('status');
  if (
    value === 'verified' ||
    value === 'verified_pending' ||
    value === 'invalid' ||
    value === 'error'
  ) {
    return value;
  }
  // An unrecognised or missing status means we cannot claim success.
  return 'error';
}

/**
 * Terminal page of the email-confirmation round trip.
 *
 * The token is consumed server-side and the browser arrives here with only an
 * outcome, so a single-use credential never lands in SPA history or a referrer
 * header. Every outcome offers a way forward — an expired link is the single
 * most common failure in this flow, and it has to lead somewhere better than an
 * apology.
 */
export default function VerifyEmail({
  status,
  onContinue,
}: {
  status: VerifyEmailStatus;
  onContinue: () => void;
}) {
  const { t } = useTranslation();
  const [email, setEmail] = useState('');
  const [resendState, setResendState] = useState<'idle' | 'sending' | 'sent' | 'failed'>('idle');

  const handleResend = async () => {
    if (!email.includes('@') || resendState === 'sending') return;
    setResendState('sending');
    try {
      await authApi.resendVerification(email);
      setResendState('sent');
    } catch {
      setResendState('failed');
    }
  };

  const copy = {
    verified: {
      heading: t('auth.emailConfirmed'),
      body: t('auth.emailConfirmedBody'),
    },
    verified_pending: {
      heading: t('auth.emailConfirmed'),
      body: "Thanks — that's your part done. An administrator still needs to approve the account, and you'll get an email the moment they do.",
    },
    invalid: {
      heading: t('auth.linkExpired'),
      body: t('auth.linkExpiredBody'),
    },
    error: {
      heading: t('frag.couldntConfirm'),
      body: t('auth.verifyErrorBody'),
    },
  }[status];

  const showResend = status === 'invalid' || status === 'error';

  return (
    <div className="min-h-dvh flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-surface">
      <div className="max-w-md w-full">
        <Card className="overflow-hidden">
          <div className="px-8 py-10">
            <div className="flex flex-col items-center text-center">
              <DravrLogo size={64} />
              <h1 className="mt-6 font-display text-2xl font-semibold text-on-surface">{copy.heading}</h1>
              <p className="mt-3 text-sm text-on-surface-variant max-w-sm">{copy.body}</p>
            </div>

            {showResend && (
              <div className="mt-8 space-y-3">
                <Input
                  id="verify-resend-email"
                  type="email"
                  label={t('auth.emailAddressLabel')}
                  autoComplete="email"
                  placeholder="you@example.com"
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                />
                <Button
                  variant="primary"
                  onClick={() => void handleResend()}
                  disabled={resendState === 'sending' || !email.includes('@')}
                  className="w-full"
                >
                  {resendState === 'sending' ? t('auth.resendSending') : t('auth.sendNewLink')}
                </Button>
                {resendState === 'sent' && (
                  <p className="text-xs text-on-surface-variant text-center" role="status">
                    {t('auth.newLinkOnItsWay')}
                  </p>
                )}
                {resendState === 'failed' && (
                  <p className="text-xs text-error text-center" role="alert">
                    {t('auth.resendFailed')}
                  </p>
                )}
              </div>
            )}

            <div className="mt-8">
              <Button variant={showResend ? 'secondary' : 'primary'} onClick={onContinue} className="w-full">
                {t('auth.continueToSignIn')}
              </Button>
            </div>
          </div>
        </Card>
      </div>
    </div>
  );
}
