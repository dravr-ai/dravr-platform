// ABOUTME: The form a data access request is written with on the web privacy pane — company, its privacy address, the athlete's identifier there
// ABOUTME: Then the law, the scope and the letter's language; writes a new draft or saves a draft's changes, checked as the server checks

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useId, useState, type FormEvent } from 'react';
import { LANGUAGE_NAMES, useTranslation } from '@pierre/i18n';
import { DATA_ACCESS_LETTER_LOCALES, type DataAccessLetterLocale, type DataAccessRequest } from '@pierre/shared-types';
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
  type DataAccessOutcome,
} from '@pierre/ui-logic';
import { useDataAccessAction } from '../../hooks/useDataAccessRequests';
import { Button, Input, Radio, Select, Textarea } from '../ui';

interface DataAccessRequestFormProps {
  /** The draft being edited; a new request when absent. */
  draft?: DataAccessRequest;
  /** The request was written or saved. */
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

export default function DataAccessRequestForm({ draft, onDone, onCancel }: DataAccessRequestFormProps) {
  const { t, i18n } = useTranslation();
  const suggestionsId = useId();
  const { run, isPending } = useDataAccessAction();
  const [form, setForm] = useState<DataAccessDraftForm>(() => initialForm(draft, i18n.language));
  const [errors, setErrors] = useState<DataAccessDraftErrors>({});
  const [failure, setFailure] = useState<string | null>(null);

  const set = <K extends keyof DataAccessDraftForm>(key: K, value: DataAccessDraftForm[K]) =>
    setForm((current) => ({ ...current, [key]: value }));

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const found = validateDataAccessDraft(form);
    setErrors(found);
    setFailure(null);
    if (Object.keys(found).length > 0) return;
    const body = dataAccessDraftBody(form);
    const outcome: DataAccessOutcome = await run(
      draft ? { kind: 'edit', id: draft.id, edit: body } : { kind: 'create', body },
    );
    if (outcome.ok) {
      onDone(outcome.request);
    } else {
      setFailure(t(outcome.errorKey));
    }
  };

  return (
    <form className="space-y-6" onSubmit={submit} noValidate data-testid="data-access-form">
      <div className="max-w-md space-y-5">
        <Input
          label={t('dataAccess.form.company')}
          helpText={t('dataAccess.form.companyHint')}
          error={errors.controller && t(errors.controller)}
          value={form.controller}
          maxLength={DATA_ACCESS_FIELD_LIMITS.controller}
          list={suggestionsId}
          autoComplete="organization"
          onChange={(event) => set('controller', event.target.value)}
          data-testid="data-access-company"
        />
        <datalist id={suggestionsId}>
          {DATA_ACCESS_COMPANY_SUGGESTIONS.map((name) => (
            <option key={name} value={name} />
          ))}
        </datalist>
        <Textarea
          rows={2}
          label={t('dataAccess.form.contact')}
          helpText={t('dataAccess.form.contactHint')}
          error={errors.controller_contact && t(errors.controller_contact)}
          value={form.controller_contact}
          maxLength={DATA_ACCESS_FIELD_LIMITS.controller_contact}
          onChange={(event) => set('controller_contact', event.target.value)}
          data-testid="data-access-contact"
        />
        <Input
          label={t('dataAccess.form.identifier')}
          helpText={t('dataAccess.form.identifierHint')}
          error={errors.athlete_identifier && t(errors.athlete_identifier)}
          value={form.athlete_identifier}
          maxLength={DATA_ACCESS_FIELD_LIMITS.athlete_identifier}
          autoComplete="email"
          onChange={(event) => set('athlete_identifier', event.target.value)}
          data-testid="data-access-identifier"
        />
      </div>

      <fieldset className="space-y-3">
        <legend className="mb-3 text-sm font-medium text-on-surface-variant">{t('dataAccess.form.law')}</legend>
        {DATA_ACCESS_LAW_OPTIONS.map((option) => (
          <Radio
            key={option.value}
            name="data-access-law"
            value={option.value}
            checked={form.law === option.value}
            onChange={() => set('law', option.value)}
            label={t(option.labelKey)}
            description={t(option.hintKey)}
            data-testid={`data-access-law-${option.value}`}
          />
        ))}
      </fieldset>

      <fieldset className="space-y-3">
        <legend className="mb-3 text-sm font-medium text-on-surface-variant">{t('dataAccess.form.scope')}</legend>
        {DATA_ACCESS_SCOPE_OPTIONS.map((option) => (
          <Radio
            key={option.value}
            name="data-access-scope"
            value={option.value}
            checked={form.scope === option.value}
            onChange={() => set('scope', option.value)}
            label={t(option.labelKey)}
            description={t(option.hintKey)}
            data-testid={`data-access-scope-${option.value}`}
          />
        ))}
      </fieldset>

      <div className="max-w-xs">
        <Select
          label={t('dataAccess.form.letterLanguage')}
          helpText={t('dataAccess.form.letterLanguageHint')}
          value={form.letter_locale}
          onChange={(event) => set('letter_locale', event.target.value as DataAccessLetterLocale)}
          options={DATA_ACCESS_LETTER_LOCALES.map((locale) => ({ value: locale, label: LANGUAGE_NAMES[locale] }))}
          data-testid="data-access-language"
        />
      </div>

      {failure && (
        <p className="text-sm text-error" role="alert" data-testid="data-access-form-failure">
          {failure}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <Button type="submit" size="sm" loading={isPending} data-testid="data-access-submit">
          {draft ? t('dataAccess.action.save') : t('dataAccess.action.create')}
        </Button>
        <Button type="button" variant="tertiary" size="sm" onClick={onCancel} disabled={isPending}>
          {t('common.cancel')}
        </Button>
      </div>
    </form>
  );
}
