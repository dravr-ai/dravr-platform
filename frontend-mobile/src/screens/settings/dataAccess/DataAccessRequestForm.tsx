// ABOUTME: The form a data access request is written with on the mobile privacy screen — company, its privacy address, the athlete's identifier there
// ABOUTME: Then the law, the scope and the letter's language as choice rows; writes a new draft or saves a draft's changes, checked as the server checks

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { Pressable, ScrollView, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { LANGUAGE_NAMES, useTranslation } from '@pierre/i18n';
import { DATA_ACCESS_LETTER_LOCALES, type DataAccessRequest } from '@pierre/shared-types';
import {
  DATA_ACCESS_COMPANY_SUGGESTIONS,
  DATA_ACCESS_FIELD_LIMITS,
  DATA_ACCESS_LAW_OPTIONS,
  DATA_ACCESS_SCOPE_OPTIONS,
  dataAccessDraftBody,
  defaultLetterLocale,
  validateDataAccessDraft,
  type DataAccessDraftErrors,
  type DataAccessDraftForm,
} from '@pierre/ui-logic';
import { Button, Input, Row } from '../../../components/ui';
import { useThemeColors } from '../../../constants/theme';
import { useDataAccessAction } from '../../../hooks/useDataAccessRequests';

interface DataAccessRequestFormProps {
  /** The draft being edited; a new request when absent. */
  draft?: DataAccessRequest;
  onDone: (request: DataAccessRequest) => void;
  onCancel: () => void;
}

function initialForm(draft: DataAccessRequest | undefined, language: string): DataAccessDraftForm {
  return draft
    ? {
        controller: draft.controller,
        controller_contact: draft.controller_contact,
        athlete_identifier: draft.athlete_identifier,
        law: draft.law,
        scope: draft.scope,
        letter_locale: draft.letter_locale,
      }
    : {
        controller: '',
        controller_contact: '',
        athlete_identifier: '',
        law: 'quebec_p39_1',
        scope: 'access_portability',
        letter_locale: defaultLetterLocale(language),
      };
}

/** One choice of a group: a row with the check on the chosen one. */
function ChoiceRow({
  title,
  subtitle,
  selected,
  last,
  onPress,
  testID,
}: {
  title: string;
  subtitle?: string;
  selected: boolean;
  last: boolean;
  onPress: () => void;
  testID: string;
}) {
  const colors = useThemeColors();
  return (
    <Row
      title={title}
      subtitle={subtitle}
      onPress={onPress}
      showChevron={false}
      last={last}
      accessibilityRole="radio"
      accessibilityState={{ selected }}
      trailing={selected ? <Feather name="check" size={18} color={colors.tokens.primary} /> : undefined}
      testID={testID}
    />
  );
}

export function DataAccessRequestForm({ draft, onDone, onCancel }: DataAccessRequestFormProps): React.JSX.Element {
  const { t, i18n } = useTranslation();
  const { run, isPending } = useDataAccessAction();
  const [form, setForm] = useState<DataAccessDraftForm>(() => initialForm(draft, i18n.language));
  const [errors, setErrors] = useState<DataAccessDraftErrors>({});
  const [failure, setFailure] = useState<string | null>(null);

  const set = <K extends keyof DataAccessDraftForm>(key: K, value: DataAccessDraftForm[K]) =>
    setForm((current) => ({ ...current, [key]: value }));

  const submit = async () => {
    const found = validateDataAccessDraft(form);
    setErrors(found);
    setFailure(null);
    if (Object.keys(found).length > 0) return;
    const body = dataAccessDraftBody(form);
    const outcome = await run(draft ? { kind: 'edit', id: draft.id, edit: body } : { kind: 'create', body });
    if (outcome.ok) onDone(outcome.request);
    else setFailure(t(outcome.errorKey));
  };

  return (
    <View className="gap-6" testID="data-access-form">
      <View className="px-4">
        <Input
          label={t('dataAccess.form.company')}
          value={form.controller}
          onChangeText={(value) => set('controller', value)}
          error={errors.controller && t(errors.controller)}
          maxLength={DATA_ACCESS_FIELD_LIMITS.controller}
          autoCapitalize="words"
          testID="data-access-company"
        />
        <Text className="-mt-2 mb-2 text-xs text-text-tertiary">{t('dataAccess.form.companyHint')}</Text>
        <ScrollView horizontal showsHorizontalScrollIndicator={false} keyboardShouldPersistTaps="handled">
          <View className="flex-row gap-2 pb-4">
            {DATA_ACCESS_COMPANY_SUGGESTIONS.map((name) => (
              <Pressable
                key={name}
                onPress={() => set('controller', name)}
                accessibilityRole="button"
                className="min-h-11 justify-center rounded-lg bg-surface-container-high px-3"
                testID={`data-access-company-${name}`}
              >
                <Text className="text-sm text-text-primary">{name}</Text>
              </Pressable>
            ))}
          </View>
        </ScrollView>
        <Input
          label={t('dataAccess.form.contact')}
          value={form.controller_contact}
          onChangeText={(value) => set('controller_contact', value)}
          error={errors.controller_contact && t(errors.controller_contact)}
          maxLength={DATA_ACCESS_FIELD_LIMITS.controller_contact}
          autoCapitalize="none"
          autoCorrect={false}
          multiline
          testID="data-access-contact"
        />
        <Text className="-mt-2 mb-4 text-xs text-text-tertiary">{t('dataAccess.form.contactHint')}</Text>
        <Input
          label={t('dataAccess.form.identifier')}
          value={form.athlete_identifier}
          onChangeText={(value) => set('athlete_identifier', value)}
          error={errors.athlete_identifier && t(errors.athlete_identifier)}
          maxLength={DATA_ACCESS_FIELD_LIMITS.athlete_identifier}
          autoCapitalize="none"
          autoCorrect={false}
          keyboardType="email-address"
          testID="data-access-identifier"
        />
        <Text className="-mt-2 text-xs text-text-tertiary">{t('dataAccess.form.identifierHint')}</Text>
      </View>

      <View accessibilityRole="radiogroup">
        <Text className="mb-1 px-4 text-sm font-medium text-text-secondary">{t('dataAccess.form.law')}</Text>
        {DATA_ACCESS_LAW_OPTIONS.map((option, index) => (
          <ChoiceRow
            key={option.value}
            title={t(option.labelKey)}
            subtitle={t(option.hintKey)}
            selected={form.law === option.value}
            last={index === DATA_ACCESS_LAW_OPTIONS.length - 1}
            onPress={() => set('law', option.value)}
            testID={`data-access-law-${option.value}`}
          />
        ))}
      </View>

      <View accessibilityRole="radiogroup">
        <Text className="mb-1 px-4 text-sm font-medium text-text-secondary">{t('dataAccess.form.scope')}</Text>
        {DATA_ACCESS_SCOPE_OPTIONS.map((option, index) => (
          <ChoiceRow
            key={option.value}
            title={t(option.labelKey)}
            subtitle={t(option.hintKey)}
            selected={form.scope === option.value}
            last={index === DATA_ACCESS_SCOPE_OPTIONS.length - 1}
            onPress={() => set('scope', option.value)}
            testID={`data-access-scope-${option.value}`}
          />
        ))}
      </View>

      <View accessibilityRole="radiogroup">
        <Text className="px-4 text-sm font-medium text-text-secondary">{t('dataAccess.form.letterLanguage')}</Text>
        <Text className="mb-1 px-4 text-xs text-text-tertiary">{t('dataAccess.form.letterLanguageHint')}</Text>
        {DATA_ACCESS_LETTER_LOCALES.map((locale, index) => (
          <ChoiceRow
            key={locale}
            title={LANGUAGE_NAMES[locale]}
            selected={form.letter_locale === locale}
            last={index === DATA_ACCESS_LETTER_LOCALES.length - 1}
            onPress={() => set('letter_locale', locale)}
            testID={`data-access-language-${locale}`}
          />
        ))}
      </View>

      <View className="gap-3 px-4">
        {failure && (
          <Text className="text-sm text-error" accessibilityRole="alert" testID="data-access-form-failure">
            {failure}
          </Text>
        )}
        <Button
          title={draft ? t('dataAccess.action.save') : t('dataAccess.action.create')}
          onPress={() => void submit()}
          loading={isPending}
          testID="data-access-submit"
        />
        <Button title={t('common.cancel')} variant="ghost" onPress={onCancel} disabled={isPending} />
      </View>
    </View>
  );
}
