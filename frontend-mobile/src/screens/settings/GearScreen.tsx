// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Settings › Shoes on the phone — the pairs the athlete declares, each with its distance, its facts, its warning and its actions
// ABOUTME: Adding or changing a pair happens in the bottom Sheet; retiring keeps a pair's distance and history, deleting needs no activity on it (carnet#711)

import React, { useState } from 'react';
import { ActivityIndicator, Alert, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';
import { useTranslation } from '@pierre/i18n';
import { distanceSymbol, formatDate, type DistanceUnit, type Translate } from '@pierre/chat-utils';
import {
  GEAR_NAME_MAX_CHARS,
  GEAR_SPORTS,
  isGearSport,
  type CreateGearRequest,
  type GearItem,
} from '@pierre/shared-types';
import {
  checkGearDraft,
  classifyApiError,
  describeApiError,
  emptyGearDraft,
  formatGearDistance,
  gearChanges,
  gearDistanceLimit,
  type GearDraft,
  type GearDraftErrors,
  gearDraftOf,
  splitGear,
  sportLabel,
  useDistanceUnit,
} from '@pierre/ui-logic';
import { spacing, useThemeColors } from '../../constants/theme';
import {
  Button,
  Checkbox,
  EmptyState,
  Input,
  PaneScrollView,
  Section,
  Sheet,
  TextTabs,
} from '../../components/ui';
import { useGearChanges, useGearLedger } from '../../hooks/useGear';
import { GearWarning } from './GearWarning';

/** The HTTP status the server refuses a delete with while activities count on the pair. */
const ATTRIBUTED_STATUS = 409;

interface GearFormProps {
  initial: GearDraft;
  submitLabel: string;
  isPending: boolean;
  /** Why the last request was refused, said above the buttons. */
  failure: string | null;
  /** The request a draft inside the server's bounds makes; a field out of them is named under it instead. */
  onSubmit: (request: Required<CreateGearRequest>) => void;
  onCancel: () => void;
  testID: string;
}

/**
 * The fields of a pair: its name, its sport, the distances, and whether it is
 * pinned. The distances are typed in the units the draft opened in, and
 * labelled with them; the sport is the text-tab row a choice in a form takes.
 */
function GearForm({ initial, submitLabel, isPending, failure, onSubmit, onCancel, testID }: GearFormProps) {
  const { t, language } = useTranslation();
  const [draft, setDraft] = useState<GearDraft>(initial);
  const [errors, setErrors] = useState<GearDraftErrors>({});
  const unitLabel = { unit: distanceSymbol(draft.unit) };
  const params = {
    characters: GEAR_NAME_MAX_CHARS,
    distance: formatGearDistance(gearDistanceLimit(draft.unit), draft.unit, language),
  };
  const error = (key: string | undefined) => (key === undefined ? undefined : t(key, params));
  const set = (change: Partial<GearDraft>) => setDraft((current) => ({ ...current, ...change }));
  const submit = () => {
    const check = checkGearDraft(draft);
    if (!check.ok) {
      setErrors(check.errors);
      return;
    }
    setErrors({});
    onSubmit(check.request);
  };

  return (
    <ScrollView keyboardShouldPersistTaps="handled" showsVerticalScrollIndicator={false} testID={testID}>
      <View className="px-4">
        <Input
          label={t('gear.form.name')}
          placeholder={t('gear.form.namePlaceholder')}
          value={draft.name}
          maxLength={GEAR_NAME_MAX_CHARS}
          onChangeText={(name) => set({ name })}
          error={error(errors.name)}
          testID={`${testID}-name`}
        />
        <Text className="mb-2 text-sm font-medium text-text-secondary">{t('gear.form.sport')}</Text>
      </View>
      <TextTabs
        className="mb-4"
        items={GEAR_SPORTS.map((sport) => ({ key: sport, label: sportLabel(t, sport) }))}
        value={draft.sport}
        onChange={(sport) => {
          if (isGearSport(sport)) set({ sport });
        }}
        testID={`${testID}-sport`}
      />
      <View className="px-4">
        <Input
          label={t('gear.form.initialDistance', unitLabel)}
          keyboardType="decimal-pad"
          value={draft.initialDistance}
          onChangeText={(initialDistance) => set({ initialDistance })}
          error={error(errors.initialDistance)}
          testID={`${testID}-initial`}
        />
        <Input
          label={t('gear.form.warnDistance', unitLabel)}
          keyboardType="decimal-pad"
          value={draft.warnDistance}
          onChangeText={(warnDistance) => set({ warnDistance })}
          error={error(errors.warnDistance)}
          testID={`${testID}-warn`}
        />
        <Text className="-mt-2 mb-4 text-xs text-text-secondary">{t('gear.form.warnHint')}</Text>
        <Checkbox
          checked={draft.pinned}
          onChange={(pinned) => set({ pinned })}
          label={t('gear.form.pinned')}
          testID={`${testID}-pinned`}
        />
        {/* Under the label, past the 20 box and its 8 gap. */}
        <Text className="mb-6 ml-7 text-xs text-text-secondary">{t('gear.form.pinnedHint')}</Text>
        {failure !== null && (
          <Text className="mb-3 text-sm text-error" accessibilityRole="alert" testID={`${testID}-failed`}>
            {failure}
          </Text>
        )}
        <View className="flex-row gap-3">
          <View className="flex-1">
            <Button
              title={t('common.cancel')}
              variant="ghost"
              onPress={onCancel}
              fullWidth
              testID={`${testID}-cancel`}
            />
          </View>
          <View className="flex-1">
            <Button
              title={submitLabel}
              onPress={submit}
              loading={isPending}
              fullWidth
              testID={`${testID}-submit`}
            />
          </View>
        </View>
      </View>
    </ScrollView>
  );
}

interface InkActionProps {
  label: string;
  onPress: () => void;
  disabled?: boolean;
  testID: string;
}

/** One of a pair's actions: an ink word with a 44 target, the way a row's own action reads. */
function InkAction({ label, onPress, disabled = false, testID }: InkActionProps) {
  return (
    <Pressable
      className={`min-h-11 justify-center ${disabled ? 'opacity-50' : ''}`}
      onPress={onPress}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityState={{ disabled }}
      testID={testID}
    >
      <Text className="text-sm font-medium text-primary">{label}</Text>
    </Pressable>
  );
}

/** The figures under a pair's name: how many activities, when last used or retired, where it warns. */
function pairFacts(t: Translate, pair: GearItem, unit: DistanceUnit, language: string): string {
  const facts = [
    pair.activity_count === 1
      ? t('gear.activityCountOne', { count: pair.activity_count })
      : t('gear.activityCountN', { count: pair.activity_count }),
  ];
  if (pair.retired_at !== null) {
    facts.push(t('gear.retiredOn', { date: formatDate(pair.retired_at, language) }));
  } else if (pair.last_used_at !== null) {
    facts.push(t('gear.lastUsed', { date: formatDate(pair.last_used_at, language) }));
  }
  facts.push(t('gear.warnAt', { distance: formatGearDistance(pair.warn_distance_meters, unit, language) }));
  return facts.join(' · ');
}

interface PairRowProps {
  pair: GearItem;
  /** The last row of its group draws no hairline under itself. */
  last: boolean;
  /** The pair's actions, on one line under its facts. */
  children: React.ReactNode;
}

/**
 * One pair, on the settings row's anatomy: the outer view pays the 16 inset,
 * the inner one carries the hairline. Its name and distance on one line, its
 * sport and badges under them, its facts, its warning once reached, then its
 * actions as ink words.
 */
function PairRow({ pair, last, children }: PairRowProps) {
  const { t, language } = useTranslation();
  const unit = useDistanceUnit();
  return (
    <View className="px-4" testID={`gear-${pair.id}`}>
      <View
        className={`pt-3 ${last ? '' : 'border-b border-border-faint'}`}
        style={last ? undefined : { borderBottomWidth: StyleSheet.hairlineWidth }}
      >
        <View className="flex-row items-baseline gap-3">
          <Text className="flex-1 text-base text-text-primary">{pair.name}</Text>
          <Text
            className="font-mono text-base tabular-nums text-text-primary"
            testID={`gear-distance-${pair.id}`}
          >
            {formatGearDistance(pair.distance_meters, unit, language)}
          </Text>
        </View>
        <View className="mt-0.5 flex-row flex-wrap">
          <Text className="text-sm text-text-secondary">{sportLabel(t, pair.sport)}</Text>
          {pair.is_default && (
            <Text
              className="text-sm font-medium text-primary"
              accessibilityHint={t('gear.defaultHint')}
              testID={`gear-default-${pair.id}`}
            >
              {` · ${t('gear.default')}`}
            </Text>
          )}
          {pair.pinned && (
            <Text className="text-sm text-text-secondary" testID={`gear-pinned-${pair.id}`}>
              {` · ${t('gear.pinned')}`}
            </Text>
          )}
        </View>
        <Text className="mt-0.5 text-xs text-text-secondary">{pairFacts(t, pair, unit, language)}</Text>
        {pair.warn_reached && <GearWarning pair={pair} />}
        <View className="flex-row flex-wrap gap-x-5">{children}</View>
      </View>
    </View>
  );
}

/**
 * The athlete's shoes. Each run, walk or hike adds its distance to the pair it
 * was done in; a pair in use can be changed, retired or — while nothing
 * counts on it — deleted, and a retired one put back in use.
 */
export function GearScreen() {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const unit = useDistanceUnit();
  const ledger = useGearLedger();
  const { create, update, retire, restore, remove } = useGearChanges();
  // The pair the sheet changes (`null` adds one) outlives the sheet's close,
  // so its title and form stay put while it slides away.
  const [sheetOpen, setSheetOpen] = useState(false);
  const [editing, setEditing] = useState<GearItem | null>(null);
  const [formKey, setFormKey] = useState(0);

  const { inUse, retired } = splitGear(ledger.data?.gear ?? []);
  const changeError = retire.error ?? restore.error;

  const openEditor = (pair: GearItem | null) => {
    create.reset();
    update.reset();
    setEditing(pair);
    setFormKey((key) => key + 1);
    setSheetOpen(true);
  };
  const closeEditor = () => setSheetOpen(false);

  const submit = (pair: GearItem | null, request: Required<CreateGearRequest>) => {
    if (pair === null) {
      create.mutate(request, { onSuccess: closeEditor });
      return;
    }
    const changes = gearChanges(pair, request);
    if (Object.keys(changes).length === 0) {
      closeEditor();
      return;
    }
    update.mutate({ gearId: pair.id, changes }, { onSuccess: closeEditor });
  };

  // A pair an activity was counted on since the list was read is refused
  // with 409: the alert then says to retire it instead.
  const confirmDelete = (pair: GearItem) => {
    const title = t('gear.deleteTitle', { name: pair.name });
    Alert.alert(title, t('gear.deleteMessage'), [
      { text: t('common.cancel'), style: 'cancel' },
      {
        text: t('common.delete'),
        style: 'destructive',
        onPress: () =>
          remove.mutate(pair.id, {
            onError: (err) =>
              Alert.alert(
                title,
                classifyApiError(err).status === ATTRIBUTED_STATUS ? t('gear.deleteLocked') : t('gear.changeFailed'),
              ),
          }),
      },
    ]);
  };

  const deleteAction = (pair: GearItem) =>
    pair.activity_count === 0 ? (
      <InkAction
        label={t('common.delete')}
        onPress={() => confirmDelete(pair)}
        disabled={remove.isPending}
        testID={`gear-delete-${pair.id}`}
      />
    ) : null;

  const renderInUse = () => {
    if (ledger.isPending) {
      return (
        <View className="items-start px-4 py-3" accessibilityLabel={t('common.loading')} testID="gear-loading">
          <ActivityIndicator color={colors.tokens.primary} />
        </View>
      );
    }
    if (ledger.isError) {
      return (
        <View className="flex-row flex-wrap items-baseline px-4" testID="gear-load-failed">
          <Text className="text-sm text-error" accessibilityRole="alert">
            {describeApiError(ledger.error, { t, fallbackKey: 'gear.loadFailed' })}
          </Text>
          <Text
            className="ml-1 text-sm font-medium text-primary"
            accessibilityRole="button"
            onPress={() => {
              void ledger.refetch();
            }}
            testID="gear-retry"
          >
            {t('common.retry')}
          </Text>
        </View>
      );
    }
    if (inUse.length === 0) {
      return (
        <EmptyState
          testID="gear-empty"
          action={{ label: t('gear.add'), onPress: () => openEditor(null), testID: 'gear-add-open' }}
        >
          {t('gear.empty')}
        </EmptyState>
      );
    }
    return (
      <View testID="gear-in-use">
        {inUse.map((pair, index) => (
          <PairRow key={pair.id} pair={pair} last={index === inUse.length - 1}>
            <InkAction label={t('common.edit')} onPress={() => openEditor(pair)} testID={`gear-edit-${pair.id}`} />
            <InkAction
              label={t('gear.retire')}
              onPress={() => retire.mutate(pair.id)}
              disabled={retire.isPending}
              testID={`gear-retire-${pair.id}`}
            />
            {deleteAction(pair)}
          </PairRow>
        ))}
      </View>
    );
  };

  const submitting = editing === null ? create : update;
  const failure = submitting.isError
    ? describeApiError(submitting.error, { t, fallbackKey: editing === null ? 'gear.addFailed' : 'gear.saveFailed' })
    : null;

  return (
    <View className="flex-1 bg-background-primary" testID="gear-screen">
      <PaneScrollView contentContainerStyle={{ paddingTop: spacing.lg, paddingBottom: spacing.xl }}>
        <Text className="px-4 pb-6 text-sm text-text-secondary">{t('gear.blurb')}</Text>
        <View className="gap-8">
          <Section
            title={t('gear.inUse')}
            testID="gear-in-use-section"
            actions={
              inUse.length > 0 ? (
                <Text
                  className="text-sm font-medium text-primary"
                  accessibilityRole="button"
                  onPress={() => openEditor(null)}
                  testID="gear-add-open"
                >
                  {t('gear.add')}
                </Text>
              ) : undefined
            }
          >
            {renderInUse()}
            {changeError && (
              <Text className="mt-2 px-4 text-sm text-error" accessibilityRole="alert" testID="gear-change-failed">
                {describeApiError(changeError, { t, fallbackKey: 'gear.changeFailed' })}
              </Text>
            )}
          </Section>
          {retired.length > 0 && (
            <Section title={t('gear.retired')} description={t('gear.retiredHint')} testID="gear-retired-section">
              <View testID="gear-retired">
                {retired.map((pair, index) => (
                  <PairRow key={pair.id} pair={pair} last={index === retired.length - 1}>
                    <InkAction
                      label={t('gear.restore')}
                      onPress={() => restore.mutate(pair.id)}
                      disabled={restore.isPending}
                      testID={`gear-restore-${pair.id}`}
                    />
                    {deleteAction(pair)}
                  </PairRow>
                ))}
              </View>
            </Section>
          )}
        </View>
      </PaneScrollView>

      <Sheet visible={sheetOpen} onClose={closeEditor} flush testID="gear-sheet">
        <Text className="mb-4 px-4 text-lg font-semibold text-text-primary" numberOfLines={1}>
          {editing === null ? t('gear.addTitle') : editing.name}
        </Text>
        {/* A fresh form on every opening, and again once the units resolve. */}
        <GearForm
          key={`${formKey}-${unit}`}
          initial={editing === null ? emptyGearDraft('run', unit) : gearDraftOf(editing, unit)}
          submitLabel={editing === null ? t('gear.add') : t('common.save')}
          isPending={submitting.isPending}
          failure={failure}
          onSubmit={(request) => submit(editing, request)}
          onCancel={closeEditor}
          testID={editing === null ? 'gear-add' : `gear-form-${editing.id}`}
        />
      </Sheet>
    </View>
  );
}
