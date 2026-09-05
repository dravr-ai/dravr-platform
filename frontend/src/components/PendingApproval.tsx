// ABOUTME: Waiting screen for accounts that cannot sign in yet — unconfirmed address, or awaiting review
// ABOUTME: The two are different situations with different next actions, so the page renders them differently
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useAuth } from '../hooks/useAuth';
import { authApi } from '../services/api';
import { Button, Card, Badge } from './ui';

// Clock icon for pending status
function ClockIcon({ className }: { className?: string }) {
  return (
    <svg className={className} fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={1.5}
        d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
      />
    </svg>
  );
}

// Pierre holistic node logo SVG
import { DravrLogo } from './DravrLogo';
import { useTranslation } from '@pierre/i18n';

/**
 * Shown to a signed-in user who cannot proceed yet. Two distinct situations land
 * here and they are not interchangeable:
 *
 * - **Address not confirmed** — the ball is in the user's court. The page leads
 *   with that and offers a resend, because telling someone to "wait for an
 *   administrator" when the actual blocker is an unopened email is how people
 *   give up on a product.
 * - **Confirmed, awaiting review** — the ball is with an operator. Nothing for
 *   the user to do, so the page says so plainly and confirms the address is done.
 *
 * `email_verified` is optional on purpose: absent means the server didn't resolve
 * it on this response, not that the address is unconfirmed. Only an explicit
 * `false` puts the page in confirm-your-email mode.
 */
export default function PendingApproval() {
  const { t } = useTranslation();
  const { user, logout } = useAuth();
  const [resendState, setResendState] = useState<'idle' | 'sending' | 'sent' | 'failed'>('idle');

  const needsEmailConfirmation = user?.email_verified === false;

  const handleResend = async () => {
    if (!user?.email || resendState === 'sending') return;
    setResendState('sending');
    try {
      await authApi.resendVerification(user.email);
      setResendState('sent');
    } catch {
      setResendState('failed');
    }
  };

  return (
    <div className="min-h-dvh flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-surface">
      <div className="max-w-md w-full">
        <Card className="overflow-hidden">
          <div className="px-8 py-10">
            {/* Logo and icon */}
            <div className="flex flex-col items-center text-center">
              <DravrLogo size={64} />

              <div className="mt-6 mb-4">
                <ClockIcon className="w-16 h-16 text-nutrition mx-auto" />
              </div>

              <h1 className="font-display text-2xl font-semibold text-on-surface">
                {needsEmailConfirmation ? t('auth.confirmYourEmailTitle') : t('auth.pendingApprovalTitle')}
              </h1>

              <p className="mt-3 text-sm text-on-surface-variant max-w-sm">
                {needsEmailConfirmation ? (
                  <>
                    {t('auth.confirmationLinkSent')}
                  </>
                ) : (
                  <>
                    {t('auth.pendingApprovalBody')}
                  </>
                )}
              </p>
            </div>

            {needsEmailConfirmation && (
              <div className="mt-6 flex flex-col items-center gap-2">
                <Button
                  variant="primary"
                  onClick={() => void handleResend()}
                  disabled={resendState === 'sending'}
                  className="w-full"
                >
                  {resendState === 'sending' ? t('auth.resendSending') : t('auth.resendLink')}
                </Button>
                {resendState === 'sent' && (
                  <p className="text-xs text-on-surface-variant" role="status">
                    {t('auth.resendSent')}
                  </p>
                )}
                {resendState === 'failed' && (
                  <p className="text-xs text-error" role="alert">
                    {t('auth.resendFailed')}
                  </p>
                )}
              </div>
            )}

            {/* Status card */}
            <div className="mt-8 bg-surface-container-low rounded-lg p-4 space-y-3">
              <div className="flex items-center justify-between">
                <span className="text-sm font-medium text-on-surface-variant">{t('auth.statusFieldLabel')}</span>
                <Badge variant="warning">
                  {needsEmailConfirmation ? t('auth.emailUnconfirmed') : t('auth.awaitingReview')}
                </Badge>
              </div>

              {user?.email_verified === true && (
                <div className="flex items-center justify-between">
                  <span className="text-sm font-medium text-on-surface-variant">
                    {t('auth.emailConfirmed')}
                  </span>
                  <Badge variant="success">{t('auth.stepDone')}</Badge>
                </div>
              )}

              {user?.email && (
                <div className="flex items-center justify-between">
                  <span className="text-sm font-medium text-on-surface-variant">{t('auth.emailFieldLabel')}</span>
                  <span className="text-sm text-on-surface-variant">{user.email}</span>
                </div>
              )}

              {user?.display_name && (
                <div className="flex items-center justify-between">
                  <span className="text-sm font-medium text-on-surface-variant">{t('auth.nameFieldLabel')}</span>
                  <span className="text-sm text-on-surface-variant">{user.display_name}</span>
                </div>
              )}
            </div>

            {/* What happens next */}
            <div className="mt-6">
              <h2 className="text-sm font-semibold text-on-surface mb-3">
                {t('auth.whatHappensNext')}
              </h2>
              <ul className="text-sm text-on-surface-variant space-y-2">
                {needsEmailConfirmation ? (
                  <>
                    <li className="flex items-start gap-2">
                      <span className="text-activity mt-0.5">•</span>
                      <span>{t('auth.openConfirmationLink')}</span>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-activity mt-0.5">•</span>
                      <span>{t('auth.activatesOnConfirm')}</span>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-activity mt-0.5">•</span>
                      <span>{t('auth.thenConnectService')}</span>
                    </li>
                  </>
                ) : (
                  <>
                    <li className="flex items-start gap-2">
                      <span className="text-activity mt-0.5">•</span>
                      <span>{t('auth.adminWillReview')}</span>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-activity mt-0.5">•</span>
                      <span>{t('auth.emailOnApproval')}</span>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-activity mt-0.5">•</span>
                      <span>{t('auth.afterApprovalAccess')}</span>
                    </li>
                  </>
                )}
              </ul>
            </div>

            {/* Sign out button */}
            <div className="mt-8">
              <Button
                variant="secondary"
                onClick={logout}
                className="w-full"
              >
                {t('auth.signOutButton')}
              </Button>
            </div>
          </div>
        </Card>
      </div>
    </div>
  );
}
