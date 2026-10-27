// ABOUTME: Landing page for /verify-email#token=… — confirms the address on a button press, never on load
// ABOUTME: Every outcome has a next action, and an expired link re-sends from itself without asking for the address
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import type { EmailConfirmationStatus } from '@pierre/shared-types';
import { authApi, emailConfirmationApi } from '../services/api';
import { Button, Card, Input } from './ui';
import { DravrLogo } from './DravrLogo';
import { useTranslation } from '@pierre/i18n';

/** Where the page is: waiting for the press, waiting for the server, or an outcome. */
type Phase = 'confirm' | 'confirming' | EmailConfirmationStatus | 'error';

type ResendState = 'idle' | 'sending' | 'sent' | 'failed';

/**
 * Terminal page of the email-confirmation round trip.
 *
 * The link carries its single-use token in the fragment, and nothing spends it
 * until the person presses the button. Mail scanners open every link in a
 * message, and some run the page's script: a confirmation fired on load would
 * spend the token before its owner saw it, and their own click would then be
 * told the link had expired.
 *
 * `token` is `null` when the link arrived without one.
 */
export default function VerifyEmail({
  token,
  onContinue,
}: {
  token: string | null;
  onContinue: () => void;
}) {
  const { t } = useTranslation();
  const [phase, setPhase] = useState<Phase>(token ? 'confirm' : 'invalid');
  const [email, setEmail] = useState('');
  const [resendState, setResendState] = useState<ResendState>('idle');

  const handleConfirm = async () => {
    if (!token || phase === 'confirming') return;
    setPhase('confirming');
    try {
      const { status } = await emailConfirmationApi.confirmEmail(token);
      setPhase(status);
    } catch {
      setPhase('error');
    }
  };

  // An expired link names its own account, so it re-sends from the token; only
  // a link the server never issued has to fall back to asking for the address.
  const handleResend = async () => {
    if (resendState === 'sending') return;
    const request =
      phase === 'expired' && token
        ? () => emailConfirmationApi.resendForLink(token)
        : email.includes('@')
          ? () => authApi.resendVerification(email)
          : null;
    if (!request) return;
    setResendState('sending');
    try {
      await request();
      setResendState('sent');
    } catch {
      setResendState('failed');
    }
  };

  const copy: Record<Phase, { heading: string; body: string }> = {
    confirm: { heading: t('auth.confirmEmailHeading'), body: t('auth.confirmEmailBody') },
    confirming: { heading: t('auth.confirmEmailHeading'), body: t('auth.confirmEmailBody') },
    verified: { heading: t('auth.emailConfirmed'), body: t('auth.emailConfirmedBody') },
    verified_pending: { heading: t('auth.emailConfirmed'), body: t('auth.emailConfirmedPendingBody') },
    expired: { heading: t('auth.linkExpired'), body: t('auth.linkExpiredBody') },
    invalid: { heading: t('auth.linkInvalid'), body: t('auth.linkInvalidBody') },
    error: { heading: t('frag.couldntConfirm'), body: t('auth.verifyErrorBody') },
  };
  const { heading, body } = copy[phase];

  const awaitingPress = phase === 'confirm' || phase === 'confirming';
  const confirmed = phase === 'verified' || phase === 'verified_pending';

  return (
    <div className="min-h-dvh flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-surface pad-safe-top">
      <div className="max-w-md w-full">
        <Card className="overflow-hidden">
          <div className="px-8 py-10">
            <div className="flex flex-col items-center text-center">
              <DravrLogo size={64} />
              <h1 className="mt-6 font-display text-2xl font-semibold text-on-surface">{heading}</h1>
              <p className="mt-3 text-sm text-on-surface-variant max-w-sm">{body}</p>
            </div>

            {awaitingPress && (
              <div className="mt-8">
                <Button size="lg"
                  variant="primary"
                  onClick={() => void handleConfirm()}
                  disabled={phase === 'confirming'}
                  className="w-full"
                >
                  {phase === 'confirming' ? t('auth.confirmingEmail') : t('auth.confirmEmailButton')}
                </Button>
              </div>
            )}

            {phase === 'error' && (
              <div className="mt-8">
                <Button size="lg" variant="primary" onClick={() => void handleConfirm()} className="w-full">
                  {t('common.tryAgain')}
                </Button>
              </div>
            )}

            {(phase === 'expired' || phase === 'invalid') && (
              <div className="mt-8 space-y-3">
                {phase === 'invalid' && (
                  <Input
                    id="verify-resend-email"
                    type="email"
                    label={t('auth.emailAddressLabel')}
                    autoComplete="email"
                    placeholder="you@example.com"
                    value={email}
                    onChange={(e) => setEmail(e.target.value)}
                  />
                )}
                <Button size="lg"
                  variant="primary"
                  onClick={() => void handleResend()}
                  disabled={
                    resendState === 'sending' ||
                    resendState === 'sent' ||
                    (phase === 'invalid' && !email.includes('@'))
                  }
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

            {!awaitingPress && (
              <div className="mt-8">
                <Button size="lg" variant={confirmed ? 'primary' : 'secondary'} onClick={onContinue} className="w-full">
                  {t('auth.continueToSignIn')}
                </Button>
              </div>
            )}
          </div>
        </Card>
      </div>
    </div>
  );
}
