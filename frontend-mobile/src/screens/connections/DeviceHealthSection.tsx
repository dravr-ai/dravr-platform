// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The phone's own health store on the Connections screen — Apple Health (this iPhone) or Health Connect: toggle, last sync, counts, delete
// ABOUTME: Read-only lane; turning it off keeps what was synced, "Delete synced data" removes it behind a confirmation and turns the toggle off

import React from 'react';
import { ActivityIndicator, Alert, Switch, Text, View } from 'react-native';
import { useTranslation } from '@pierre/i18n';
import { formatSyncTime } from '@pierre/ui-logic';
import { Row, Section, switchColors } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { useDeviceHealth, type DeviceHealthFailure } from '../../hooks/useDeviceHealth';

const FAILURE_KEYS: Record<DeviceHealthFailure, string> = {
  declined: 'providers.deviceHealth.declined',
  enable: 'providers.deviceHealth.enableFailed',
  sync: 'providers.deviceHealth.syncFailed',
  delete: 'providers.deviceHealth.deleteFailed',
};

/** Nothing on a phone with no store (web, or Health Connect not installed). */
export function DeviceHealthSection(): React.JSX.Element | null {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const row = useDeviceHealth();

  if (!row.visible || row.source === null) return null;
  const name = t(row.source === 'apple_health' ? 'providers.deviceHealth.appleHealth' : 'providers.deviceHealth.healthConnect');

  const status = row.syncing
    ? t('providers.deviceHealth.syncing')
    : !row.enabled
      ? t('providers.deviceHealth.off')
      : row.lastSyncAt
        ? t('providers.deviceHealth.lastSync', { time: formatSyncTime(row.lastSyncAt, language) })
        : t('providers.deviceHealth.neverSynced');

  const confirmDelete = () => {
    Alert.alert(t('providers.deviceHealth.deleteTitle'), t('providers.deviceHealth.deleteBody', { name }), [
      { text: t('common.cancel'), style: 'cancel' },
      {
        text: t('providers.deviceHealth.deleteConfirm'),
        style: 'destructive',
        onPress: () => {
          void row.deleteSyncedData();
        },
      },
    ]);
  };

  const toggle = (on: boolean) => {
    void (on ? row.turnOn() : row.turnOff());
  };

  return (
    <Section
      title={t('providers.deviceHealth.sectionTitle')}
      description={t('providers.deviceHealth.description')}
      testID="device-health-section"
    >
      <Row
        title={name}
        subtitle={status}
        last
        testID="device-health-row"
        trailing={
          <View className="flex-row items-center gap-2">
            {(row.busy || row.syncing) && <ActivityIndicator size="small" color={colors.tokens.primary} />}
            <Switch
              testID="device-health-switch"
              accessibilityLabel={t('providers.deviceHealth.toggleLabel', { name })}
              value={row.enabled}
              onValueChange={toggle}
              disabled={row.busy}
              {...switchColors(colors.tokens, row.enabled)}
            />
          </View>
        }
      />
      {row.lastCounts && row.enabled && !row.syncing && (
        <Text className="px-4 pt-1 text-sm text-text-secondary" testID="device-health-counts">
          {[
            t('providers.deviceHealth.countWorkouts', { count: row.lastCounts.activities }),
            t('providers.deviceHealth.countNights', { count: row.lastCounts.nights }),
            t('providers.deviceHealth.countDays', { count: row.lastCounts.days }),
            t('providers.deviceHealth.countEntries', { count: row.lastCounts.self_reports }),
          ].join(' · ')}
        </Text>
      )}
      {row.enabled && row.lastSyncAt && !row.received && !row.syncing && (
        <Text className="px-4 pt-1 text-sm text-text-secondary" testID="device-health-nothing-arrived">
          {t('providers.deviceHealth.nothingArrived', { name })}
        </Text>
      )}
      {row.failure && (
        <Text className="px-4 pt-1 text-sm text-error" accessibilityRole="alert" testID="device-health-error">
          {t(FAILURE_KEYS[row.failure], { name })}
        </Text>
      )}
      <Text
        className="px-4 pt-2.5 text-sm font-medium text-error self-start"
        onPress={row.busy ? undefined : confirmDelete}
        accessibilityRole="button"
        accessibilityState={{ disabled: row.busy }}
        testID="device-health-delete"
      >
        {t('providers.deviceHealth.deleteAction')}
      </Text>
    </Section>
  );
}
