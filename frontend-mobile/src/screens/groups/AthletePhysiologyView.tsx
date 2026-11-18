// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coached athlete's thresholds per sport, shown in Group info in place of the roster, where their coach sets one (carnet#1561)
// ABOUTME: Worded for the coach; a value saved is stored as set by a coach and the athlete is told, through the one physiology writer

import React, { useState } from 'react';
import { ActivityIndicator, ScrollView, Text, TouchableOpacity, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useTranslation } from '@pierre/i18n';
import {
  COACH_PHYSIOLOGY_KEY,
  PHYSIOLOGY_ANCHOR_LABEL_KEY,
  PHYSIOLOGY_EDIT_KEY,
  SPORT_FAMILY_LABEL_KEY,
} from '@pierre/shared-constants';
import type { PhysiologyAnchor, SportFamily } from '@pierre/shared-types';
import { describeApiError, physiologyAnchorFacts, type AnchorFormTarget } from '@pierre/ui-logic';
import { useThemeColors } from '../../constants/theme';
import { Button, Row, Section } from '../../components/ui';
import { PhysiologyAnchorForm } from '../../components/PhysiologyAnchorForm';
import { useAthletePhysiology, useAthletePhysiologyEdit } from '../../hooks/useCoachPhysiology';
import type { GroupMember } from '../../types';

export interface AthletePhysiologyViewProps {
  /** The athlete whose thresholds are open. */
  member: GroupMember;
  /** The group the caller coaches them in. */
  groupId: string;
  /** Back to Group info. */
  onBack: () => void;
}

/**
 * The thresholds a coached athlete's workouts are set on, as their coach
 * reads them from Group info, and the form that sets one. Drawn inside the
 * Group info sheet rather than over it, so no second sheet stacks on the
 * first; the form takes the list's place while it is open. An athlete who
 * stopped sharing with the coach, and a read that failed, each say so.
 */
export function AthletePhysiologyView({ member, groupId, onBack }: AthletePhysiologyViewProps) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const { read, isLoading, isError, error, refetch } = useAthletePhysiology(member.user_id, groupId);
  const save = useAthletePhysiologyEdit(member.user_id, groupId);
  const [target, setTarget] = useState<AnchorFormTarget | null>(null);
  const [saved, setSaved] = useState(false);
  const name = member.display_name ?? t('app.unknownMember');

  const open = (next: AnchorFormTarget) => {
    setSaved(false);
    setTarget(next);
  };

  const rows = (sport: SportFamily, anchors: PhysiologyAnchor[]) =>
    anchors.map((anchor, index) => {
      const label = t(PHYSIOLOGY_ANCHOR_LABEL_KEY[anchor.anchor]);
      return (
        <Row
          key={anchor.anchor}
          title={label}
          subtitle={physiologyAnchorFacts(t, anchor, language).join(' · ')}
          trailing={
            <View className="max-w-[55%] items-end py-2">
              <Text className="text-right text-base tabular-nums text-text-primary">{anchor.value_text}</Text>
              <Text className="text-right text-sm text-text-secondary">{anchor.source_text}</Text>
            </View>
          }
          onPress={() => open({ mode: 'edit', sport, anchor })}
          accessibilityRole="button"
          accessibilityLabel={`${t(PHYSIOLOGY_EDIT_KEY.edit)} · ${label} · ${t(SPORT_FAMILY_LABEL_KEY[sport])}`}
          last={index === anchors.length - 1}
          testID={`athlete-physiology-${sport}-${anchor.anchor}`}
        />
      );
    });

  let body: React.ReactNode = null;
  if (isLoading) {
    body = (
      <View className="py-10 items-center">
        <ActivityIndicator size="large" color={colors.pierre.violet} />
      </View>
    );
  } else if (isError) {
    body = (
      <View className="py-6 items-center" testID="athlete-physiology-error">
        <Text className="text-sm text-text-secondary text-center mb-3">
          {describeApiError(error, { t, fallbackKey: COACH_PHYSIOLOGY_KEY.loadFailed })}
        </Text>
        <Button title={t('common.retry')} variant="secondary" onPress={refetch} />
      </View>
    );
  } else if (read?.status === 'consentRevoked') {
    body = (
      <Text className="py-6 text-sm text-text-secondary text-center" testID="athlete-physiology-revoked">
        {t(COACH_PHYSIOLOGY_KEY.consentRevoked, { name })}
      </Text>
    );
  } else if (read?.status === 'ok' && target !== null) {
    body = (
      <PhysiologyAnchorForm
        target={target}
        offered={read.physiology.offered}
        onDone={() => setTarget(null)}
        onSave={async (anchorSave) => {
          await save.mutateAsync(anchorSave);
          setSaved(true);
        }}
      />
    );
  } else if (read?.status === 'ok') {
    const { sports } = read.physiology;
    body = (
      <View className="gap-6" testID="athlete-physiology">
        <Text className="text-xs text-text-tertiary">{t(COACH_PHYSIOLOGY_KEY.blurb, { name })}</Text>
        {sports.length === 0 ? (
          <Text className="text-sm text-text-secondary" testID="athlete-physiology-empty">
            {t(COACH_PHYSIOLOGY_KEY.empty, { name })}
          </Text>
        ) : (
          sports.map((family) => (
            <Section key={family.sport} title={t(SPORT_FAMILY_LABEL_KEY[family.sport])}>
              {rows(family.sport, family.anchors)}
            </Section>
          ))
        )}
        {saved && (
          <Text className="text-sm text-text-secondary" accessibilityRole="text" testID="athlete-physiology-saved">
            {t(COACH_PHYSIOLOGY_KEY.saved, { name })}
          </Text>
        )}
        <Button
          title={t(PHYSIOLOGY_EDIT_KEY.add)}
          variant="secondary"
          onPress={() => open({ mode: 'add' })}
          testID="athlete-physiology-add"
        />
      </View>
    );
  }

  return (
    <ScrollView testID="athlete-physiology-view" keyboardShouldPersistTaps="handled">
      <View className="flex-row items-center mb-4">
        <TouchableOpacity
          className="p-2 -ml-2 mr-1"
          onPress={target === null ? onBack : () => setTarget(null)}
          accessibilityRole="button"
          accessibilityLabel={t('common.back')}
          testID="athlete-physiology-back"
        >
          <Feather name="chevron-left" size={22} color={colors.text.primary} />
        </TouchableOpacity>
        <Text className="flex-1 text-lg font-bold text-text-primary" numberOfLines={1}>
          {t(COACH_PHYSIOLOGY_KEY.title, { name })}
        </Text>
      </View>
      {body}
    </ScrollView>
  );
}
