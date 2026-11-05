// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info's "what your human coach can do" — the member's calendar-write and wellness consents to the group's coach
// ABOUTME: One block on the coach relationship, shown with or without a link; a wellness revoke asks first, as it deletes

import { useId, useState } from 'react';
import type { CoachConsentsResponse } from '@pierre/shared-types';
import {
  COACH_CONSENTS_KEYS,
  DELEGATION_CONSENT_KEYS,
  WELLNESS_WIDENED_NOTICE_KEYS,
  coachPlatformName,
  consentChangeConfirm,
  consentNoteKey,
  consentPlatformName,
  delegationRefusalKey,
  offeredConsents,
  type DelegationConsent,
} from '@pierre/shared-constants';
import { refusalProvider, refusalReason } from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';
import { useSetCoachConsents } from '../../hooks/useGroups';
import { Button, Checkbox, ConfirmDialog, useErrorToast, useSuccessToast } from '../ui';

interface CoachConsentsSectionProps {
  groupId: string;
  /** What the member lets the coach do now, and what the relationship offers. */
  consents: CoachConsentsResponse;
  /** The group's coach as the server names them. */
  coach: string;
}

/**
 * The member's switches for what the group's coach may do beyond reading
 * their training. They live on the member's coach relationship, so there is
 * one block for the group, whether the member reaches the coaching platform
 * through a link or through a connection of their own. The calendar switch
 * says the step the athlete also takes on the platform itself: making the
 * coach their Coach there. A wellness grant given before the consent widened
 * to every app the member connects is told so above the switches, with its
 * two answers: keep it (granted again) or stop (the ordinary revoke).
 */
export default function CoachConsentsSection({ groupId, consents, coach }: CoachConsentsSectionProps) {
  const { t } = useTranslation();
  const noticeTitleId = useId();
  const showSuccess = useSuccessToast();
  const showError = useErrorToast();
  const { setConsents, isPending } = useSetCoachConsents(groupId);
  // A change on its way to the server, shown as made until the server answers.
  const [changing, setChanging] = useState<Partial<Record<DelegationConsent, boolean>>>({});
  // A revoke waiting on the member's confirm, which it asks before deleting what was read.
  const [revoking, setRevoking] = useState<DelegationConsent | null>(null);
  const revokeConfirm = revoking ? consentChangeConfirm(revoking, false) : undefined;
  const offered = offeredConsents(consents);

  const change = async (consent: DelegationConsent, granted: boolean) => {
    setChanging((held) => ({ ...held, [consent]: granted }));
    try {
      await setConsents({ [consent]: granted });
      showSuccess(t('delegation.consentsSavedToast'));
    } catch (err) {
      showError(t(delegationRefusalKey(refusalReason(err)), { platform: coachPlatformName(refusalProvider(err)) }));
    } finally {
      // The consents now say where each stands: granted, or unchanged after a refusal.
      setChanging((held) => ({ ...held, [consent]: undefined }));
    }
  };

  if (offered.length === 0) return null;

  return (
    <div className="space-y-3" data-testid="coach-consents">
      <p className="text-sm text-on-surface-variant">{t(COACH_CONSENTS_KEYS.hint, { coach })}</p>
      {consents.wellness_notice_due && (
        <div
          role="note"
          aria-labelledby={noticeTitleId}
          className="space-y-2 rounded-lg border border-info/30 bg-info/10 p-3"
          data-testid="coach-consent-wellness-notice"
        >
          <p id={noticeTitleId} className="text-sm font-medium text-on-info-container">
            {t(WELLNESS_WIDENED_NOTICE_KEYS.title)}
          </p>
          <p className="text-sm text-on-info-container">
            {t(WELLNESS_WIDENED_NOTICE_KEYS.body, { coach, platform: consentPlatformName('wellness_read') })}
          </p>
          <div className="flex flex-wrap items-center gap-2">
            {/* Keeping is a fresh grant: the server stamps it, and the notice goes with its answer. */}
            <Button
              variant="tertiary"
              size="sm"
              disabled={isPending}
              onClick={() => void change('wellness_read', true)}
              data-testid="coach-consent-wellness-keep"
            >
              {t(WELLNESS_WIDENED_NOTICE_KEYS.keep)}
            </Button>
            <Button
              variant="tertiary"
              size="sm"
              className="text-error"
              disabled={isPending}
              onClick={() => setRevoking('wellness_read')}
              data-testid="coach-consent-wellness-stop"
            >
              {t(WELLNESS_WIDENED_NOTICE_KEYS.stop)}
            </Button>
          </div>
        </div>
      )}
      {offered.map((consent) => {
        const note = consentNoteKey(consents, consent);
        const platform = consentPlatformName(consent);
        return (
          <div key={consent} className="space-y-1">
            <Checkbox
              label={t(DELEGATION_CONSENT_KEYS[consent].label, { platform, coach })}
              description={t(DELEGATION_CONSENT_KEYS[consent].hint, { platform, coach })}
              checked={changing[consent] ?? consents[consent]}
              disabled={isPending}
              onChange={(e) => {
                const granted = e.target.checked;
                if (consentChangeConfirm(consent, granted)) setRevoking(consent);
                else void change(consent, granted);
              }}
              data-testid={`coach-consent-${consent}`}
            />
            {/* Past the control, under its words: the platform step, or why a granted consent syncs nothing. */}
            {consent === 'calendar_write' && (
              <p className="pl-7 text-xs text-on-surface-variant" data-testid="coach-consent-calendar-step">
                {t(COACH_CONSENTS_KEYS.calendarStep, { platform, coach })}
              </p>
            )}
            {note && (
              <p className="pl-7 text-xs text-on-surface-variant" data-testid={`coach-consent-note-${consent}`}>
                {t(note, { platform })}
              </p>
            )}
          </div>
        );
      })}
      <ConfirmDialog
        isOpen={revokeConfirm !== undefined}
        onClose={() => setRevoking(null)}
        onConfirm={() => {
          if (revoking) void change(revoking, false);
          setRevoking(null);
        }}
        title={revokeConfirm ? t(revokeConfirm.title) : ''}
        message={revokeConfirm && revoking ? t(revokeConfirm.body, { platform: consentPlatformName(revoking), coach }) : ''}
        confirmLabel={revokeConfirm ? t(revokeConfirm.confirm) : undefined}
        variant="danger"
      />
    </div>
  );
}
