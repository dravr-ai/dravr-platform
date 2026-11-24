// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info's "what your human coach can do" — the member's calendar-write and wellness consents to the group's coach
// ABOUTME: One block on the coach relationship, shown with or without a link; a wellness revoke asks first; health records ride wellness

import React, { useEffect, useState } from 'react';
import { Alert, Switch, Text, View, type ViewStyle } from 'react-native';
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
import { formatDate } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { useThemeColors } from '../../constants/theme';
import { ALERT_TONE, Button, Row, switchColors } from '../../components/ui';
import { useSetCoachConsents } from '../../hooks/useGroups';
import type { CoachConsentsResponse } from '../../types';

/** Cancels the host sheet's side inset, so a `Row` pays its own (see GroupInfoSheet). */
const CANCEL_PANEL_INSET: ViewStyle = { marginHorizontal: -16 };

export interface CoachConsentsSectionProps {
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
 * through a link or through a connection of their own. The calendar row says
 * the step the athlete also takes on the platform itself: making the coach
 * their Coach there. A wellness grant given before the consent widened to
 * every app the member connects is told so above the switches, with its two
 * answers: keep it (granted again) or stop (the ordinary revoke). While the
 * member shares their wellness, a row of its own lets the coach also read
 * their health records, until revoked or for 30 days.
 */
export function CoachConsentsSection({ groupId, consents, coach }: CoachConsentsSectionProps) {
  const { t, i18n } = useTranslation();
  const colors = useThemeColors();
  const { setConsents, isPending } = useSetCoachConsents(groupId);
  // A change on its way to the server, shown as made until the server answers.
  const [changing, setChanging] = useState<Partial<Record<DelegationConsent, boolean>>>({});
  const offered = offeredConsents(consents);

  // A change the server took is let go once the consents passed in show it:
  // the cache holds the answer before this block's parent renders it, and
  // letting go sooner flicks the switch back for a frame.
  useEffect(() => {
    setChanging((held) => {
      const shown = (Object.keys(held) as DelegationConsent[]).filter((key) => held[key] === consents[key]);
      if (shown.length === 0) return held;
      const next = { ...held };
      for (const key of shown) delete next[key];
      return next;
    });
  }, [consents]);

  const change = async (consent: DelegationConsent, granted: boolean) => {
    setChanging((held) => ({ ...held, [consent]: granted }));
    try {
      const answered = await setConsents({ [consent]: granted });
      if (answered[consent] === granted) return;
    } catch (err) {
      Alert.alert(
        t('common.error'),
        t(delegationRefusalKey(refusalReason(err)), { platform: coachPlatformName(refusalProvider(err)) }),
      );
    }
    // Refused, or answered otherwise: the consents say where it stands.
    setChanging((held) => ({ ...held, [consent]: undefined }));
  };

  /** A turn of a consent, behind a confirm when it deletes what was read. */
  const askThenChange = (consent: DelegationConsent, granted: boolean) => {
    const ask = consentChangeConfirm(consent, granted);
    if (!ask) {
      void change(consent, granted);
      return;
    }
    Alert.alert(t(ask.title), t(ask.body, { platform: consentPlatformName(consent), coach }), [
      { text: t('common.cancel'), style: 'cancel' },
      { text: t(ask.confirm), style: 'destructive', onPress: () => void change(consent, granted) },
    ]);
  };

  // The health-records flag, its own row: the change in flight, and the window a grant takes.
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
    } catch (err) {
      Alert.alert(
        t('common.error'),
        t(delegationRefusalKey(refusalReason(err)), { platform: coachPlatformName(refusalProvider(err)) }),
      );
    } finally {
      setRecordsChanging(undefined);
    }
  };

  const pickWindow = (window: 'forever' | 'thirty') => {
    setRecordsWindow(window);
    // A standing grant takes its new window at once: granted again.
    if (consents.sensitive_t2_read) void changeRecords(true, window);
  };

  if (offered.length === 0) return null;

  return (
    <View testID="coach-consents">
      <Text className="text-sm text-text-secondary pb-2">{t(COACH_CONSENTS_KEYS.hint, { coach })}</Text>
      {consents.wellness_notice_due && (
        <View
          className={`mb-3 gap-2 rounded-xl border ${ALERT_TONE.info.edge} ${ALERT_TONE.info.fill} p-3`}
          accessibilityRole="summary"
          testID="coach-consent-wellness-notice"
        >
          <Text className={`text-sm font-semibold ${ALERT_TONE.info.ink}`}>{t(WELLNESS_WIDENED_NOTICE_KEYS.title)}</Text>
          <Text className={`text-sm ${ALERT_TONE.info.ink}`}>
            {t(WELLNESS_WIDENED_NOTICE_KEYS.body, { coach, platform: consentPlatformName('wellness_read') })}
          </Text>
          <View className="flex-row flex-wrap gap-2">
            {/* Keeping is a fresh grant: the server stamps it, and the notice goes with its answer. */}
            <Button
              title={t(WELLNESS_WIDENED_NOTICE_KEYS.keep)}
              variant="secondary"
              onPress={() => void change('wellness_read', true)}
              disabled={isPending}
              testID="coach-consent-wellness-keep"
            />
            <Button
              title={t(WELLNESS_WIDENED_NOTICE_KEYS.stop)}
              variant="ghost"
              onPress={() => askThenChange('wellness_read', false)}
              disabled={isPending}
              testID="coach-consent-wellness-stop"
            />
          </View>
        </View>
      )}
      <View style={CANCEL_PANEL_INSET}>
        {offered.map((consent, index) => {
          const platform = consentPlatformName(consent);
          const label = t(DELEGATION_CONSENT_KEYS[consent].label, { platform, coach });
          const hint = t(DELEGATION_CONSENT_KEYS[consent].hint, { platform, coach });
          const note = consentNoteKey(consents, consent);
          const lines = [hint];
          // Under its words: the platform step, or why a granted consent syncs nothing.
          if (consent === 'calendar_write') lines.push(t(COACH_CONSENTS_KEYS.calendarStep, { platform, coach }));
          if (note) lines.push(t(note, { platform }));
          return (
            <Row
              key={consent}
              title={label}
              subtitle={lines.join('\n')}
              trailing={
                <Switch
                  value={changing[consent] ?? consents[consent]}
                  onValueChange={(granted) => askThenChange(consent, granted)}
                  disabled={isPending}
                  {...switchColors(colors.tokens, changing[consent] ?? consents[consent])}
                  accessibilityLabel={label}
                  testID={`coach-consent-${consent}`}
                />
              }
              last={index === offered.length - 1 && !recordsShown}
              testID={`coach-consent-row-${consent}`}
            />
          );
        })}
        {recordsShown && (
          <Row
            title={t(SENSITIVE_T2_FLAG_KEYS.label, { coach })}
            subtitle={[
              t(SENSITIVE_T2_FLAG_KEYS.hint, { coach }),
              ...(consents.sensitive_t2_read && consents.sensitive_t2_read_until
                ? [
                    t(SENSITIVE_T2_FLAG_KEYS.until, {
                      date: formatDate(consents.sensitive_t2_read_until, i18n.language),
                    }),
                  ]
                : []),
            ].join('\n')}
            trailing={
              <Switch
                value={recordsGranted}
                onValueChange={(granted) => void changeRecords(granted, recordsWindow)}
                disabled={isPending}
                {...switchColors(colors.tokens, recordsGranted)}
                accessibilityLabel={t(SENSITIVE_T2_FLAG_KEYS.label, { coach })}
                testID="coach-consent-sensitive-t2"
              />
            }
            last
            testID="coach-consent-row-sensitive-t2"
          />
        )}
      </View>
      {recordsShown && (
        <View className="flex-row flex-wrap gap-2 pt-2" testID="coach-consent-sensitive-window">
          <Button
            title={t(SENSITIVE_T2_FLAG_KEYS.forever)}
            variant={recordsWindow === 'forever' ? 'secondary' : 'ghost'}
            onPress={() => pickWindow('forever')}
            disabled={isPending}
            testID="coach-consent-sensitive-forever"
          />
          <Button
            title={t(SENSITIVE_T2_FLAG_KEYS.thirtyDays)}
            variant={recordsWindow === 'thirty' ? 'secondary' : 'ghost'}
            onPress={() => pickWindow('thirty')}
            disabled={isPending}
            testID="coach-consent-sensitive-thirty"
          />
        </View>
      )}
    </View>
  );
}
