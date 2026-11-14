// ABOUTME: Landing page for a mailed invitation (/accept-invite#token=…) — the invitee chooses their password
// ABOUTME: Redeems the single-use token once; an expired or used link leads to sign-up or sign-in, never a dead end
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { invitationsApi } from '../services/api';
import { Button, Input, RevealButton } from './ui';
import { DravrLogo } from './DravrLogo';
import AnalyticsConsentOptIn from './AnalyticsConsentOptIn';
import { useTranslation } from '@pierre/i18n';
import type { InvitationLinkKind } from '../utils/tokenLink';

type Stage = 'form' | 'invalid' | 'accepted';

interface AcceptInvitationProps {
  /** The token from the link, or `null` when the link carried none. */
  token: string | null;
  /**
   * What the link says it is: a team-issued coach seat words the page for a
   * coach. The server decides what the account is granted, never this.
   */
  kind?: InvitationLinkKind;
  /** Go to the sign-in form. */
  onSignIn: () => void;
  /** Go to the create-account form, the way in when the invitation is spent. */
  onRegister: () => void;
}

/**
 * Where an invitation link lands. The address is the one the invitation was
 * sent to, so the form asks only for a name and a password; the account is
 * created active the moment it is submitted.
 *
 * A link that no longer works (expired, used, replaced by a newer one) is the
 * common failure, and it leads to the create-account form: the operator's
 * pre-approval stands, so signing up there still lands the account active.
 *
 * A coach-seat link (carnet#1524) carries the coach's seat: the page says so
 * before the account is made, and the done screen says the seat is already
 * active. One that no longer works says so plainly and that the Dravr team
 * sends a new one; a plain sign-up would carry no seat.
 */
export default function AcceptInvitation({ token, kind = 'account', onSignIn, onRegister }: AcceptInvitationProps) {
  const { t } = useTranslation();
  const coachSeat = kind === 'coach_seat';
  const [stage, setStage] = useState<Stage>(token ? 'form' : 'invalid');
  const [displayName, setDisplayName] = useState('');
  const [password, setPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [showPassword, setShowPassword] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState('');
  // The usage-analytics opt-in, unticked until ticked (carnet#1366).
  const [analyticsConsent, setAnalyticsConsent] = useState(false);
  // The address the account signs in with: the invitee never typed it, so the
  // done screen names it before they reach the sign-in form.
  const [email, setEmail] = useState('');

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError('');
    if (!token) {
      setStage('invalid');
      return;
    }
    if (password.length < 8) {
      setError(t('app.passwordTooShort'));
      return;
    }
    if (password !== confirmPassword) {
      setError(t('app.passwordsDoNotMatch'));
      return;
    }

    setIsLoading(true);
    try {
      const account = await invitationsApi.acceptInvitation(
        token,
        password,
        displayName.trim() || undefined,
        analyticsConsent,
      );
      setEmail(account.email);
      setStage('accepted');
    } catch (err: unknown) {
      const apiError = err as {
        response?: { status?: number; data?: { message?: string; error?: string } };
      };
      if (apiError.response?.status === 404) {
        setStage('invalid');
      } else {
        setError(
          apiError.response?.data?.message ||
            apiError.response?.data?.error ||
            t('app.registrationFailedRetry'),
        );
      }
    } finally {
      setIsLoading(false);
    }
  };

  if (stage !== 'form') {
    const accepted = stage === 'accepted';
    const title = accepted
      ? t(coachSeat ? 'humanCoach.seatInvite.doneTitle' : 'auth.registrationSuccess')
      : t(coachSeat ? 'humanCoach.seatInvite.expiredTitle' : 'auth.linkExpired');
    return (
      <div className="min-h-dvh flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-surface pad-safe-top">
        <div className="max-w-md w-full">
          <div className="rounded-xl border ghost-border overflow-hidden relative bg-surface-container-lowest">
            <div className="px-8 py-10">
              <div className="flex flex-col items-center text-center">
                <DravrLogo size={64} />
                <h1 className="mt-6 text-xl font-bold text-on-surface tracking-tight">{title}</h1>
                {accepted && email && (
                  <p className="mt-3 text-sm text-on-surface" data-testid="accepted-email">
                    <span className="text-on-surface-variant">{t('auth.emailAddressLabel')} </span>
                    <span className="font-medium">{email}</span>
                  </p>
                )}
                {accepted && (
                  <p className="mt-3 text-sm text-on-surface-variant max-w-sm">
                    {t(coachSeat ? 'humanCoach.seatInvite.doneBody' : 'auth.thenConnectService')}
                  </p>
                )}
                {!accepted && coachSeat && (
                  <p className="mt-3 text-sm text-on-surface-variant max-w-sm" data-testid="invite-expired-body">
                    {t('humanCoach.seatInvite.expiredBody')}
                  </p>
                )}
              </div>
              <div className="mt-8 space-y-3">
                {!accepted && !coachSeat && (
                  <Button size="lg" variant="primary" onClick={onRegister} className="w-full">
                    {t('auth.createAccount')}
                  </Button>
                )}
                <Button
                  size="lg"
                  variant={accepted || coachSeat ? 'primary' : 'secondary'}
                  onClick={onSignIn}
                  className="w-full"
                >
                  {accepted ? t('auth.continueToSignIn') : t('auth.backToSignIn')}
                </Button>
              </div>
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-dvh flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-surface pad-safe-top">
      <div className="max-w-md w-full">
        <div className="rounded-xl border ghost-border overflow-hidden relative bg-surface-container-lowest">
          <div className="px-8 py-10 space-y-6">
            <div className="flex flex-col items-center">
              <DravrLogo size={64} />
              <h1 className="mt-4 text-xl font-bold text-on-surface tracking-tight">
                {t(coachSeat ? 'humanCoach.seatInvite.title' : 'auth.createAccountTitle')}
              </h1>
              <p className="mt-1 text-sm text-on-surface-variant text-center">
                {t(coachSeat ? 'humanCoach.seatInvite.subtitle' : 'auth.joinDravr')}
              </p>
              {coachSeat && (
                <p className="mt-3 text-xs text-on-surface-variant text-center" data-testid="invite-email-locked">
                  {t('humanCoach.seatInvite.emailLocked')}
                </p>
              )}
            </div>

            <form className="space-y-5" onSubmit={handleSubmit}>
              {error && (
                <div
                  role="alert"
                  aria-live="polite"
                  className="bg-error/10 border border-error/30 text-error px-4 py-3 rounded-lg text-sm"
                >
                  {error}
                </div>
              )}

              <div className="space-y-4">
                <Input
                  id="invite-display-name"
                  name="displayName"
                  type="text"
                  label={t('auth.displayNameLabel')}
                  autoComplete="name"
                  placeholder={t('auth.displayNamePlaceholder')}
                  value={displayName}
                  onChange={(e) => setDisplayName(e.target.value)}
                  variant="dark"
                />

                <Input
                  id="invite-password"
                  name="password"
                  type={showPassword ? 'text' : 'password'}
                  label={t('auth.passwordLabel')}
                  autoComplete="new-password"
                  required
                  placeholder={t('auth.newPasswordPlaceholderSignUp')}
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  variant="dark"
                  rightIcon={
                    <RevealButton
                      revealed={showPassword}
                      onToggle={() => setShowPassword(!showPassword)}
                      label={showPassword ? t('auth.hidePassword') : t('auth.showPassword')}
                    />
                  }
                />

                <Input
                  id="invite-confirm-password"
                  name="confirmPassword"
                  type={showPassword ? 'text' : 'password'}
                  label={t('auth.confirmPasswordLabel')}
                  autoComplete="new-password"
                  required
                  placeholder={t('auth.confirmPasswordPlaceholder')}
                  value={confirmPassword}
                  onChange={(e) => setConfirmPassword(e.target.value)}
                  variant="dark"
                />
              </div>

              <AnalyticsConsentOptIn checked={analyticsConsent} onChange={setAnalyticsConsent} />

              <Button size="lg" type="submit" variant="gradient" loading={isLoading} className="w-full">
                {isLoading ? t('auth.creatingAccount') : t('auth.createAccountButton')}
              </Button>

              <div className="text-center">
                <button
                  type="button"
                  onClick={onSignIn}
                  className="text-sm text-primary hover:text-primary-fixed-dim font-medium transition-colors"
                >
                  {t('auth.haveAccountSignIn')}
                </button>
              </div>
            </form>
          </div>
        </div>
      </div>
    </div>
  );
}
