// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The card a /groups/join/<code> link opens on, signed in or not, before any form (carnet#1649)
// ABOUTME: Who invited, to which group and what joining shares — then one filled button; a dead link says so and carries on

import { useEffect, useState } from 'react';
import { apiErrorStatus, inviteCardNamesGroupBelow, inviteCardTitle, inviteDeadLink, inviteShares } from '@pierre/ui-logic';
import { formatDate } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import type { GroupInvitePreview } from '@pierre/shared-types';
import { useInvitePreview } from '../hooks/useInvitePreview';
import type { CardJoinOutcome } from '../hooks/useHeldGroupInvite';
import { dropCarriedGroupInvite } from '../utils/groupInvite';
import { Button } from './ui';
import { DravrLogo } from './DravrLogo';

interface InviteCardProps {
  /** The code the link carried; the card reads it once. */
  code: string;
  /** The viewer has a session: the button joins, never opens a form. */
  signedIn: boolean;
  /** Signed out: the filled button — the create-account form. */
  onCreateAccount: () => void;
  /** Signed out: the ink button — the sign-in form. */
  onSignIn: () => void;
  /** Signed in: the yes. Hold, then join; answers how it came out. */
  onJoin: () => Promise<CardJoinOutcome>;
  /** The card is answered, or the link is dead: the page carries on. */
  onDone: () => void;
}

function InviteShell({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-dvh flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-surface pad-safe-top">
      <div className="max-w-md w-full">
        <div
          className="rounded-xl border ghost-border overflow-hidden relative bg-surface-container-lowest"
          data-testid="invite-card"
        >
          <div className="px-8 py-10 space-y-6">{children}</div>
        </div>
      </div>
    </div>
  );
}

/** What a dead or unreadable link says, with the way on. */
function InviteMessage({
  message,
  testId,
  retry,
  onContinue,
}: {
  message: string;
  testId: string;
  /** Present for a failure worth asking again; a dead link has none. */
  retry?: () => void;
  onContinue?: () => void;
}) {
  const { t } = useTranslation();
  return (
    <InviteShell>
      <div className="flex flex-col items-center text-center">
        <DravrLogo size={56} />
        <p className="mt-6 text-sm text-on-surface" role="status" data-testid={testId}>
          {message}
        </p>
      </div>
      <div className="flex flex-col items-center gap-2">
        {retry && (
          <Button size="lg" variant="primary" className="w-full" onClick={retry} data-testid="invite-card-retry">
            {t('humanCoach.inviteCard.retry')}
          </Button>
        )}
        {onContinue && (
          <Button
            size="lg"
            variant="tertiary"
            className="w-full"
            onClick={onContinue}
            data-testid="invite-card-continue"
          >
            {t('humanCoach.inviteCard.continueWithout')}
          </Button>
        )}
      </div>
    </InviteShell>
  );
}

function LiveInvite({
  preview,
  signedIn,
  onCreateAccount,
  onSignIn,
  onJoin,
  onDone,
  onDead,
}: Omit<InviteCardProps, 'code'> & {
  preview: GroupInvitePreview;
  onDead: (why: 'dead' | 'not_for_account') => void;
}) {
  const { t, language } = useTranslation();
  const [joining, setJoining] = useState(false);
  const [failed, setFailed] = useState(false);
  // The inviter by name when they have one; what joining shares is said of
  // the group's coach, never the inviter (ADR-002).
  const title = inviteCardTitle(preview);
  const shares = inviteShares(preview);

  const join = async () => {
    if (joining) return;
    setJoining(true);
    setFailed(false);
    try {
      const outcome = await onJoin();
      if (outcome === 'failed') {
        setFailed(true);
      } else if (outcome === 'dead' || outcome === 'not_for_account') {
        onDead(outcome);
      } else {
        onDone();
      }
    } finally {
      setJoining(false);
    }
  };

  return (
    <InviteShell>
      <div className="flex flex-col items-center text-center">
        {preview.inviter_initials !== null && (
          <span
            aria-hidden="true"
            className="mb-4 flex h-14 w-14 select-none items-center justify-center rounded-full bg-primary-container text-lg font-semibold text-on-primary-container"
            data-testid="invite-card-avatar"
          >
            {preview.inviter_initials}
          </span>
        )}
        <h1 className="font-display text-xl font-semibold text-on-surface" data-testid="invite-card-title">
          {t(title.key, title.values)}
        </h1>
        {inviteCardNamesGroupBelow(preview) && (
          <p className="mt-1 font-display text-lg text-on-surface" data-testid="invite-card-group">
            {preview.group_name}
          </p>
        )}
        <p className="mt-3 max-w-sm text-sm text-on-surface-variant" data-testid="invite-card-shares">
          {t(shares.key, shares.values)}
        </p>
      </div>

      <div className="flex flex-col items-center gap-2">
        {signedIn ? (
          <Button
            size="lg"
            variant="primary"
            className="w-full"
            loading={joining}
            onClick={() => void join()}
            data-testid="invite-card-join"
          >
            {joining ? t('humanCoach.inviteCard.joining') : t('humanCoach.inviteCard.join')}
          </Button>
        ) : (
          <>
            <Button
              size="lg"
              variant="primary"
              className="w-full"
              onClick={onCreateAccount}
              data-testid="invite-card-create"
            >
              {t('humanCoach.inviteCard.create')}
            </Button>
            <Button
              size="lg"
              variant="tertiary"
              className="w-full"
              onClick={onSignIn}
              data-testid="invite-card-have-account"
            >
              {t('humanCoach.inviteCard.haveAccount')}
            </Button>
          </>
        )}
        {failed && (
          <p className="text-xs text-error" role="alert" data-testid="invite-card-join-failed">
            {t('humanCoach.joinStrip.failed')}
          </p>
        )}
      </div>

      {preview.expires_at && (
        <p className="text-center font-mono text-xs text-on-surface-variant" data-testid="invite-card-valid-until">
          {t('humanCoach.inviteCard.validUntil', { date: formatDate(preview.expires_at, language) })}
        </p>
      )}
    </InviteShell>
  );
}

/**
 * The invite card. It reads the link once (every read counts as an "opened"
 * for the coach), and nothing is held or joined before the person answers it:
 * a signed-out visitor opens a form from it, a signed-in one joins from it.
 * A link that no longer works is said to be so, its code is dropped so it is
 * never held, and the person carries on into the app.
 */
export default function InviteCard(props: InviteCardProps) {
  const { code, onDone } = props;
  const { t } = useTranslation();
  const preview = useInvitePreview(code, true);
  // How the person's own answer ended the link: dead, or a coach link their
  // account may not redeem.
  const [endedOnJoin, setEndedOnJoin] = useState<'dead' | 'not_for_account' | null>(null);
  const deadAfterJoin = endedOnJoin !== null;

  const status = preview.data?.status;
  const unknownCode = preview.isError && apiErrorStatus(preview.error) === 404;
  const linkIsDead = unknownCode || (status !== undefined && status !== 'live') || deadAfterJoin;

  // A dead link is never held: drop the carried code as soon as it is known.
  useEffect(() => {
    if (linkIsDead) dropCarriedGroupInvite(code);
  }, [linkIsDead, code]);

  if (preview.isError && !unknownCode) {
    const rateLimited = apiErrorStatus(preview.error) === 429;
    return (
      <InviteMessage
        testId="invite-card-error"
        message={t(rateLimited ? 'humanCoach.inviteCard.tooMany' : 'humanCoach.inviteCard.loadFailed')}
        retry={() => void preview.refetch()}
        onContinue={onDone}
      />
    );
  }
  if (unknownCode) {
    return (
      <InviteMessage
        testId="invite-card-dead"
        message={t('humanCoach.inviteCard.unknown')}
        onContinue={onDone}
      />
    );
  }
  if (preview.data === undefined) {
    return (
      <div className="min-h-dvh flex items-center justify-center bg-surface pad-safe-top">
        <div className="text-center" role="status" aria-live="polite" data-testid="invite-card-loading">
          <div aria-hidden="true" className="animate-spin rounded-full h-12 w-12 border-b-2 border-primary mx-auto" />
          <p className="mt-4 text-on-surface-variant">{t('common.loading')}</p>
        </div>
      </div>
    );
  }
  if (endedOnJoin === 'not_for_account') {
    return (
      <InviteMessage
        testId="invite-card-not-for-account"
        message={t('humanCoach.inviteCard.coachLinkNotForAccount')}
        onContinue={onDone}
      />
    );
  }
  if (preview.data.status !== 'live' || deadAfterJoin) {
    const deadStatus = preview.data.status === 'live' ? 'inactive' : preview.data.status;
    const dead = inviteDeadLink(deadStatus, preview.data.inviter_name);
    return (
      <InviteMessage
        testId="invite-card-dead"
        message={t(dead.key, dead.values)}
        onContinue={onDone}
      />
    );
  }
  return <LiveInvite {...props} preview={preview.data} onDead={setEndedOnJoin} />;
}
