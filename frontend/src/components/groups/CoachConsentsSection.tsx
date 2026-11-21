// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info's "what your human coach can do" — the member's calendar-write and wellness consents to the group's coach
// ABOUTME: One block on the coach relationship, shown with or without a link; a wellness revoke asks first; health records ride wellness

import { useId, useState } from 'react';
import type { CoachConsentsResponse } from '@pierre/shared-types';
import {
  COACH_CONSENTS_KEYS,
  DELEGATION_CONSENT_KEYS,
  SENSITIVE_T2_FLAG_KEYS,
  SENSITIVE_WINDOW_DAYS,
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
import { Button, Checkbox, ConfirmDialog, Select, useErrorToast, useSuccessToast, ALERT_TONE } from '../ui';

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
 * two answers: keep it (granted again) or stop (the ordinary revoke). While
 * the member shares their wellness, a control of its own lets the coach also
 * read their health records, until revoked or for 30 days.
 */
export default function CoachConsentsSection({ groupId, consents, coach }: CoachConsentsSectionProps) {
  const { t, i18n } = useTranslation();
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

  // The health-records flag, its own control: the change in flight, and the window a grant takes.
  const [recordsChanging, setRecordsChanging] = useState<boolean | undefined>(undefined);
  const [recordsWindow, setRecordsWindow] = useState<'forever' | 'thirty'>(
    consents.sensitive_t2_read_until ? 'thirty' : 'forever',
  );
  const recordsShown = consents.sensitive_t2_read_supported && consents.wellness_read;
  const recordsGranted = recordsChanging ?? consents.sensitive_t2_read;

  const changeRecords = async (granted: boolean, window: 'forever' | 'thirty') => {
    setRecordsChanging(granted);
    try {
      await setConsents(
        granted
          ? { sensitive_t2_read: true, ...(window === 'thirty' ? { sensitive_days: SENSITIVE_WINDOW_DAYS } : {}) }
          : { sensitive_t2_read: false },
      );
      showSuccess(granted ? t(SENSITIVE_T2_FLAG_KEYS.granted, { coach }) : t('delegation.consentsSavedToast'));
    } catch (err) {
      showError(t(delegationRefusalKey(refusalReason(err)), { platform: coachPlatformName(refusalProvider(err)) }));
    } finally {
      setRecordsChanging(undefined);
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
          className={`${ALERT_TONE.info.tone} space-y-2 rounded-lg border p-3`}
          data-testid="coach-consent-wellness-notice"
        >
          <p id={noticeTitleId} className="text-sm font-medium">
            {t(WELLNESS_WIDENED_NOTICE_KEYS.title)}
          </p>
          <p className="text-sm">
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
      {recordsShown && (
        <div className="space-y-2 pl-7" data-testid="coach-consent-sensitive_t2_read">
          <Checkbox
            label={t(SENSITIVE_T2_FLAG_KEYS.label, { coach })}
            description={t(SENSITIVE_T2_FLAG_KEYS.hint, { coach })}
            checked={recordsGranted}
            disabled={isPending}
            onChange={(e) => void changeRecords(e.target.checked, recordsWindow)}
            data-testid="coach-consent-sensitive-t2"
          />
          <Select
            size="sm"
            aria-label={t(SENSITIVE_T2_FLAG_KEYS.label, { coach })}
            value={recordsWindow}
            disabled={isPending}
            options={[
              { value: 'forever', label: t(SENSITIVE_T2_FLAG_KEYS.forever) },
              { value: 'thirty', label: t(SENSITIVE_T2_FLAG_KEYS.thirtyDays) },
            ]}
            onChange={(e) => {
              const window = e.target.value === 'thirty' ? 'thirty' : 'forever';
              setRecordsWindow(window);
              // A standing grant takes its new window at once: granted again.
              if (consents.sensitive_t2_read) void changeRecords(true, window);
            }}
            data-testid="coach-consent-sensitive-window"
          />
          {consents.sensitive_t2_read && consents.sensitive_t2_read_until && (
            <p className="text-xs text-on-surface-variant" data-testid="coach-consent-sensitive-until">
              {t(SENSITIVE_T2_FLAG_KEYS.until, {
                date: new Date(consents.sensitive_t2_read_until).toLocaleDateString(i18n.language),
              })}
            </p>
          )}
        </div>
      )}
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
