// ABOUTME: Onboarding step — a coach creates their group and leaves with the athlete invite link and its QR code
// ABOUTME: Coach access is never granted here (ADR-018): without it the group is made coachless and the step says so

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useState } from 'react';
import QRCode from 'qrcode';
import type { CoachingGroup } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { chatApi, groupsApi } from '../services/api';
import { Button, Input } from './ui';
import OnboardingShell from './OnboardingShell';

/**
 * How long the onboarding athlete invite stays valid. Longer than the 7 days a
 * chat-issued invite gets: a coach shares this one with a whole roster, and
 * five athletes rarely all join within a week.
 */
const ONBOARDING_INVITE_DAYS = 30;

/** Where the group setup stands: still to name, or made and ready to share. */
type Phase =
  | { kind: 'name' }
  | { kind: 'share'; group: CoachingGroup; link: string };

/**
 * The coach's group step: name the group, then share the athlete invite.
 *
 * The group is created with the coach as its human coach when they hold coach
 * access (`manages_roster`); the server decides, never this screen. A group
 * that comes back without a coach is shown as access pending, with the way to
 * ask for it. The coach's chat thread for the group is opened here too, since
 * a group nobody has a thread for is a group the coach cannot find.
 */
export default function OnboardingCoachGroup({
  userDisplayName,
  onComplete,
}: {
  userDisplayName?: string | null;
  onComplete: (status: 'complete' | 'skipped') => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState('');
  const [phase, setPhase] = useState<Phase>({ kind: 'name' });
  const [created, setCreated] = useState<CoachingGroup | null>(null);
  const [working, setWorking] = useState(false);
  const [failed, setFailed] = useState(false);

  const create = async () => {
    const trimmed = name.trim();
    if (!trimmed || working) return;
    setWorking(true);
    setFailed(false);
    try {
      // A retry after a later call failed reuses the group already made rather
      // than making a second one.
      const group =
        created ?? (await groupsApi.createGroup({ name: trimmed, coach_is_me: true }));
      setCreated(group);
      await chatApi.createConversation({ group_id: group.id, agent_id: group.agent_id });
      const invite = await groupsApi.createInvite(group.id, {
        expires_in_days: ONBOARDING_INVITE_DAYS,
      });
      const link = `${window.location.origin}/groups/join/${encodeURIComponent(invite.code)}`;
      setPhase({ kind: 'share', group, link });
    } catch {
      setFailed(true);
    } finally {
      setWorking(false);
    }
  };

  if (phase.kind === 'share') {
    return (
      <OnboardingShell heading={t('onboarding.groupInviteHeading')}>
        <InviteShare group={phase.group} link={phase.link} onDone={() => onComplete('complete')} />
      </OnboardingShell>
    );
  }

  return (
    <OnboardingShell
      heading={
        userDisplayName
          ? t('onboarding.welcomeNamed', { name: userDisplayName })
          : t('onboarding.groupHeading')
      }
    >
      <p className="mt-3 text-sm text-on-surface-variant text-center">{t('humanCoach.groupStepIntro')}</p>
      <form
        className="mt-8 space-y-6"
        onSubmit={(e) => {
          e.preventDefault();
          void create();
        }}
      >
        <Input
          label={t('onboarding.groupNameLabel')}
          placeholder={t('onboarding.groupNamePlaceholder')}
          value={name}
          maxLength={100}
          onChange={(e) => setName(e.target.value)}
          error={failed ? t('onboarding.groupCreateFailed') : undefined}
          data-testid="onboarding-group-name"
        />
        <div className="space-y-3">
          <Button
            type="submit"
            variant="primary"
            disabled={working || name.trim() === ''}
            className="w-full"
          >
            {working ? t('onboarding.groupCreating') : t('onboarding.groupCreate')}
          </Button>
          <button
            type="button"
            onClick={() => onComplete('skipped')}
            disabled={working}
            className="w-full text-sm font-medium text-on-surface-variant hover:text-on-surface underline-offset-2 hover:underline transition-colors"
          >
            {t('onboarding.groupLater')}
          </button>
        </div>
      </form>
    </OnboardingShell>
  );
}

/** The invite link, its QR code, and the honest coach-access state. */
function InviteShare({
  group,
  link,
  onDone,
}: {
  group: CoachingGroup;
  link: string;
  onDone: () => void;
}) {
  const { t } = useTranslation();
  const [qr, setQr] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const canShare = typeof navigator !== 'undefined' && typeof navigator.share === 'function';

  useEffect(() => {
    let live = true;
    // The QR ground stays white in both themes: a phone camera needs it.
    QRCode.toDataURL(link, { margin: 1, width: 220 })
      .then((url) => {
        if (live) setQr(url);
      })
      // No QR is no loss: the link beside it is the same invite.
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [link]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(link);
      setCopied(true);
    } catch {
      // The link stays on screen to copy by hand.
    }
  };

  return (
    <>
      <p className="mt-3 text-sm text-on-surface-variant text-center">
        {t('onboarding.groupInviteIntro')}
      </p>

      {qr ? (
        <div className="mt-8 flex justify-center">
          <img
            src={qr}
            alt={t('onboarding.groupInviteQrAlt')}
            width={220}
            height={220}
            className="rounded-md bg-white"
            data-testid="onboarding-group-qr"
          />
        </div>
      ) : null}

      <p
        className="mt-6 break-all text-center font-mono text-sm text-on-surface"
        data-testid="onboarding-group-link"
      >
        {link}
      </p>

      <div className="mt-6 flex gap-3">
        <Button variant="secondary" onClick={() => void copy()} className="flex-1">
          {copied ? t('onboarding.groupLinkCopied') : t('onboarding.groupCopyLink')}
        </Button>
        {canShare ? (
          <Button
            variant="secondary"
            onClick={() => void navigator.share({ title: group.name, url: link }).catch(() => {})}
            className="flex-1"
          >
            {t('onboarding.groupShare')}
          </Button>
        ) : null}
      </div>

      {group.coach_user_id ? null : (
        <div
          className="mt-8 border-t ghost-border-faint pt-6"
          data-testid="onboarding-group-access-pending"
        >
          <h2 className="font-display font-semibold text-lg text-on-surface">
            {t('humanCoach.accessPendingHeading')}
          </h2>
          <p className="mt-2 text-sm text-on-surface-variant">
            {t('humanCoach.accessPendingBody')}
          </p>
          <a
            href="mailto:support@dravr.ai"
            className="mt-3 inline-block text-sm font-medium text-primary underline-offset-2 hover:underline"
          >
            {t('onboarding.groupAccessContact')}
          </a>
        </div>
      )}

      <Button variant="primary" onClick={onDone} className="mt-8 w-full">
        {t('onboarding.groupDone')}
      </Button>
    </>
  );
}
