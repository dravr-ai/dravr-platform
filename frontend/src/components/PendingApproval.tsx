// ABOUTME: Waiting screen for accounts that cannot sign in yet — unconfirmed address, or awaiting review
// ABOUTME: The two are different situations with different next actions; a signup code approves either at once
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useState, type FormEvent } from 'react';
import { useAuth } from '../hooks/useAuth';
import { authApi, userApi } from '../services/api';
import { describeApiError, describeEmailTombstone, inviteJourneyPromise, isInvalidSignupCode } from '@pierre/ui-logic';
import { useOnlineStatus } from '../hooks/useOnlineStatus';
import { useInviteJourney } from '../hooks/useInviteJourney';
import { Button, Card, Badge, Input } from './ui';

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

/** How often the waiting page re-reads the session for an approval. */
const PENDING_STATUS_POLL_MS = 15_000;

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
 * - **Confirmed, awaiting review** — the ball is with an admin. Nothing for
 *   the user to do, so the page says so plainly and confirms the address is done.
 *
 * `email_verified` is optional on purpose: absent means the server didn't resolve
 * it on this response, not that the address is unconfirmed. Only an explicit
 * `false` puts the page in confirm-your-email mode.
 *
 * A third reason rides on either: the address belonged to a Dravr account that
 * was deleted (`email_tombstone`). Nobody reviews that one — the athlete
 * chooses to start fresh with it, once the address is confirmed as theirs.
 */
export default function PendingApproval() {
  const { t, language } = useTranslation();
  const online = useOnlineStatus();
  const { user, logout, updateUser } = useAuth();
  const [resendState, setResendState] = useState<'idle' | 'sending' | 'sent' | 'failed'>('idle');
  const [clearing, setClearing] = useState(false);
  const [clearError, setClearError] = useState<string | null>(null);
  const [showSignupCode, setShowSignupCode] = useState(false);
  const [signupCode, setSignupCode] = useState('');
  const [redeeming, setRedeeming] = useState(false);
  const [redeemError, setRedeemError] = useState<string | null>(null);
  // Set once a code was accepted but the account is still not in: the field
  // gives way to what happens next. A held group invite approves nothing, so
  // the account may be waiting on review rather than on its address.
  const [redeemed, setRedeemed] = useState<{ groupJoined: boolean; inviteHeld: boolean } | null>(null);

  // An approval or a confirmed address lands on the server, not here: re-read
  // the session while the page is up, and whenever the tab comes back, so the
  // athlete moves on without a reload. Failures are ignored; the next tick
  // tries again.
  const userStatus = user?.user_status;
  const emailVerified = user?.email_verified;
  useEffect(() => {
    let cancelled = false;
    const check = () => {
      authApi
        .getSession()
        .then(({ user: fresh }) => {
          if (cancelled) return;
          if (fresh.user_status !== userStatus || fresh.email_verified !== emailVerified) {
            updateUser({ user_status: fresh.user_status, email_verified: fresh.email_verified });
          }
        })
        .catch(() => {});
    };
    const onVisible = () => {
      if (document.visibilityState === 'visible') check();
    };
    const timer = window.setInterval(check, PENDING_STATUS_POLL_MS);
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      document.removeEventListener('visibilitychange', onVisible);
    };
  }, [userStatus, emailVerified, updateUser]);

  const needsEmailConfirmation = user?.email_verified === false;
  // The invite this account joins once it is in, and whether an administrator
  // reviews it first, as the session says (carnet#1649, #1650).
  const invite = useInviteJourney();
  const reviewed = user?.activation === 'review';
  const tombstone = user?.email_tombstone;
  // A tombstoned address the athlete has proven: theirs to start fresh with.
  const offersClear = tombstone !== undefined && !needsEmailConfirmation;

  const handleClear = async () => {
    if (!tombstone || clearing) return;
    setClearing(true);
    setClearError(null);
    try {
      const { user_status } = await userApi.clearImportTombstone(tombstone.tombstone_id);
      // An approval rule may admit the account at once; otherwise it now waits
      // for an ordinary review.
      updateUser({ user_status, email_tombstone: undefined });
    } catch (err) {
      setClearError(describeApiError(err, { online, t, fallbackKey: 'errors.unknown' }));
    } finally {
      setClearing(false);
    }
  };

  const handleRedeem = async (e: FormEvent) => {
    e.preventDefault();
    const code = signupCode.trim();
    if (!code || redeeming || !online) return;
    setRedeeming(true);
    setRedeemError(null);
    try {
      const { user_status, group_joined, group_invite_held } = await userApi.redeemSignupCode(code);
      if (user_status === 'active') {
        // The account is in: the app leaves this screen on the new status.
        updateUser({ user_status });
      } else if (group_invite_held === true) {
        // A group invite code: the server holds it, and the account joins
        // the group once it is let in (carnet#1648). Nothing has joined yet.
        setRedeemed({ groupJoined: false, inviteHeld: true });
      } else {
        // Accepted, but the address is not confirmed yet: the confirmation
        // link activates it.
        setRedeemed({ groupJoined: group_joined === true, inviteHeld: false });
      }
    } catch (err) {
      // The server says which refusal it was: not a code, already active,
      // already used one, too many tries.
      setRedeemError(
        isInvalidSignupCode(err)
          ? t('auth.signupCodeInvalid')
          : describeApiError(err, { online, t, fallbackKey: 'errors.unknown' })
      );
    } finally {
      setRedeeming(false);
    }
  };

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
    <div className="min-h-dvh flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-surface pad-safe-top">
      <div className="max-w-md w-full">
        <Card className="overflow-hidden">
          <div className="px-8 py-10">
            {/* Logo and icon */}
            <div className="flex flex-col items-center text-center">
              <DravrLogo size={64} />

              <div className="mt-6 mb-4">
                <ClockIcon className="w-16 h-16 text-warning mx-auto" />
              </div>

              <h1 className="font-display text-2xl font-semibold text-on-surface">
                {needsEmailConfirmation ? t('auth.confirmYourEmailTitle') : t('auth.pendingApprovalTitle')}
              </h1>

              <p className="mt-3 text-sm text-on-surface-variant max-w-sm">
                {needsEmailConfirmation ? (
                  <>
                    {t('auth.confirmationLinkSent')}
                  </>
                ) : tombstone ? (
                  <span data-testid="pending-email-tombstoned">
                    {describeEmailTombstone(tombstone, { language, t })}
                  </span>
                ) : (
                  <>
                    {t('auth.pendingApprovalBody')}
                  </>
                )}
              </p>
              {invite !== null && !tombstone && (
                <p
                  className="mt-3 text-sm font-medium text-on-surface max-w-sm"
                  data-testid="pending-invite-promise"
                >
                  {(() => {
                    const promise = inviteJourneyPromise(
                      { inviter_name: invite.inviterName, group_name: invite.groupName },
                      reviewed,
                    );
                    return t(promise.key, promise.values);
                  })()}
                </p>
              )}
              {needsEmailConfirmation && tombstone && (
                <p
                  className="mt-2 text-sm text-on-surface-variant max-w-sm"
                  data-testid="pending-email-tombstoned-verify-first"
                >
                  {t('auth.pendingEmailTombstonedVerifyFirst')}
                </p>
              )}
            </div>

            {offersClear && (
              <div className="mt-6 flex flex-col items-center gap-2">
                <Button size="lg"
                  variant="primary"
                  onClick={() => void handleClear()}
                  loading={clearing}
                  className="w-full"
                  data-testid="pending-email-tombstoned-clear"
                >
                  {t('auth.pendingEmailTombstonedClear')}
                </Button>
                {clearError && (
                  <p className="text-xs text-error" role="alert">
                    {clearError}
                  </p>
                )}
              </div>
            )}

            {needsEmailConfirmation && (
              <div className="mt-6 flex flex-col items-center gap-2">
                <Button size="lg"
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

            {/* A signup code from Dravr or a coach skips the wait (carnet#1128).
                A deleted account's address is refused a code, so the field is
                not offered beside the tombstone. */}
            {tombstone === undefined && (
              <div className="mt-6" data-testid="pending-signup-code">
                {redeemed ? (
                  <div
                    role="status"
                    className="bg-surface-container-low border ghost-border text-on-surface px-4 py-3 rounded-lg text-sm space-y-1"
                  >
                    {/* A held invite on a confirmed address still waits on
                        review: telling them to confirm it would send them
                        looking for a mail that never comes. */}
                    <p>
                      {redeemed.inviteHeld && !needsEmailConfirmation
                        ? t('auth.pendingApprovalMessage')
                        : redeemed.inviteHeld && reviewed
                          ? t('auth.signupCodeAcceptedConfirmEmailReview')
                          : t('auth.signupCodeAcceptedConfirmEmail')}
                    </p>
                    {redeemed.groupJoined && <p>{t('auth.signupCodeJoinedGroup')}</p>}
                  </div>
                ) : showSignupCode ? (
                  <form onSubmit={(e) => void handleRedeem(e)} className="space-y-3">
                    <Input
                      id="pendingSignupCode"
                      name="signupCode"
                      type="text"
                      label={t('auth.signupCodeLabel')}
                      autoComplete="off"
                      autoCapitalize="characters"
                      spellCheck={false}
                      placeholder={t('auth.signupCodePlaceholder')}
                      helpText={t('auth.signupCodeHint')}
                      value={signupCode}
                      onChange={(e) => setSignupCode(e.target.value)}
                    />
                    {redeemError && (
                      <p className="text-xs text-error" role="alert">
                        {redeemError}
                      </p>
                    )}
                    <Button
                      size="lg"
                      type="submit"
                      variant="primary"
                      loading={redeeming}
                      disabled={!signupCode.trim() || !online}
                      className="w-full"
                    >
                      {redeeming ? t('auth.signupCodeApplying') : t('auth.signupCodeApply')}
                    </Button>
                  </form>
                ) : (
                  <div className="text-center">
                    <button
                      type="button"
                      onClick={() => setShowSignupCode(true)}
                      className="text-sm text-primary hover:text-primary-fixed-dim font-medium transition-colors"
                    >
                      {t('auth.haveSignupCode')}
                    </button>
                  </div>
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

            {/* What happens next — nobody reviews a tombstoned address, so the
                review steps would promise a wait that is not coming. */}
            {!offersClear && (
            <div className="mt-6">
              <h2 className="text-sm font-semibold text-on-surface mb-3">
                {t('auth.whatHappensNext')}
              </h2>
              <ul className="text-sm text-on-surface-variant space-y-2">
                {needsEmailConfirmation ? (
                  <>
                    <li className="flex items-start gap-2">
                      <span className="text-primary mt-0.5">•</span>
                      <span>{t('auth.openConfirmationLink')}</span>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-primary mt-0.5">•</span>
                      {/* Under review, confirming is not what activates the account. */}
                      <span>{t(reviewed ? 'auth.adminWillReview' : 'auth.activatesOnConfirm')}</span>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-primary mt-0.5">•</span>
                      <span>{t('auth.thenConnectService')}</span>
                    </li>
                  </>
                ) : (
                  <>
                    <li className="flex items-start gap-2">
                      <span className="text-primary mt-0.5">•</span>
                      <span>{t('auth.adminWillReview')}</span>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-primary mt-0.5">•</span>
                      <span>{t('auth.emailOnApproval')}</span>
                    </li>
                    <li className="flex items-start gap-2">
                      <span className="text-primary mt-0.5">•</span>
                      <span>{t('auth.afterApprovalAccess')}</span>
                    </li>
                  </>
                )}
              </ul>
            </div>
            )}

            {/* Sign out button */}
            <div className="mt-8">
              <Button size="lg"
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
