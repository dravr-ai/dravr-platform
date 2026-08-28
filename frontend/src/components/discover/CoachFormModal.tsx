// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach edit form — title, description, system prompt, category, data context, tool budget
// ABOUTME: A modal over whatever mounted it; the caller owns the form state and the save/delete requests

import {
  MIN_MAX_TOOL_ITERATIONS,
  MAX_MAX_TOOL_ITERATIONS,
  DEFAULT_MAX_TOOL_ITERATIONS,
} from '@pierre/shared-constants';
import type { CoachFormData } from './coachForm';
import { Select, Textarea, Radio } from '../ui';
import { useTranslation } from '@pierre/i18n';

/**
 * Hold the typed budget inside the range the server accepts. A cleared or
 * non-numeric box yields `null` — the user asking to inherit again — which the
 * update request sends as an explicit `null` so a stored pin is actually
 * cleared instead of silently preserved.
 */
function clampToolIterations(raw: string): number | null {
  const parsed = Number.parseInt(raw, 10);
  if (Number.isNaN(parsed)) return null;
  return Math.min(MAX_MAX_TOOL_ITERATIONS, Math.max(MIN_MAX_TOOL_ITERATIONS, parsed));
}

interface CoachFormModalProps {
  isOpen: boolean;
  formData: CoachFormData;
  onFormDataChange: (data: CoachFormData) => void;
  onSubmit: () => void;
  onClose: () => void;
  isSubmitting: boolean;
  submitError: boolean;
  /** Offered as "Delete this coach" under the form when the mount owns deletion. */
  onDelete?: () => void;
}

export default function CoachFormModal({
  isOpen,
  formData,
  onFormDataChange,
  onSubmit,
  onClose,
  isSubmitting,
  submitError,
  onDelete,
}: CoachFormModalProps) {
  const { t } = useTranslation();
  if (!isOpen) return null;

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!formData.title.trim() || !formData.system_prompt.trim()) return;
    onSubmit();
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/50 backdrop-blur-sm"
        onClick={onClose}
      />
      {/* Modal Content */}
      <div className="relative bg-surface rounded-2xl shadow-2xl max-w-lg w-full mx-4 max-h-[90vh] overflow-y-auto">
        <div className="p-6">
          {/* Close button */}
          <button
            onClick={onClose}
            className="absolute top-4 right-4 p-2 text-on-surface-variant hover:text-on-surface-variant hover:bg-surface-container-high rounded-lg transition-colors"
            aria-label={t('chat.close')}
          >
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>

          <div className="text-center mb-6">
            <div className="w-12 h-12 bg-primary/10 rounded-xl flex items-center justify-center mx-auto mb-4">
              <svg className="w-6 h-6 text-primary" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0z" />
              </svg>
            </div>
            <h2 className="text-xl font-semibold text-on-surface mb-2">{t('chat.editCoachTitle')}</h2>
            <p className="text-on-surface-variant text-sm">{t('chat.coachFormEditHint')}</p>
          </div>

          <form onSubmit={handleSubmit} className="space-y-4">
            <div>
              <label className="block text-sm font-medium text-on-surface-variant mb-1">
                {t('chat.coachNameLabel')}
              </label>
              <input
                type="text"
                placeholder="e.g., Marathon Training Coach"
                value={formData.title}
                onChange={(e) => onFormDataChange({ ...formData, title: e.target.value })}
                className="w-full px-3 py-2 text-sm border ghost-border rounded-lg focus:outline-none focus:ring-2 focus:ring-primary focus:border-transparent"
                required
              />
            </div>

            <div>
              <label className="block text-sm font-medium text-on-surface-variant mb-1">
                {t('chat.descriptionLabel')} <span className="text-on-surface-variant">(optional)</span>
              </label>
              <input
                type="text"
                placeholder={t('chat.descriptionPlaceholder')}
                value={formData.description}
                onChange={(e) => onFormDataChange({ ...formData, description: e.target.value })}
                className="w-full px-3 py-2 text-sm border ghost-border rounded-lg focus:outline-none focus:ring-2 focus:ring-primary focus:border-transparent"
              />
            </div>

            <Textarea
              label={t('chat.systemPromptLabel')}
              placeholder={t('chat.systemPromptPlaceholder')}
              value={formData.system_prompt}
              onChange={(e) => onFormDataChange({ ...formData, system_prompt: e.target.value })}
              rows={4}
              required
              helpText={
                formData.system_prompt
                  ? `~${Math.ceil(formData.system_prompt.length / 4)} tokens (${((Math.ceil(formData.system_prompt.length / 4) / 128000) * 100).toFixed(1)}% of context)`
                  : undefined
              }
            />

            <Select
              label={t('chat.categoryLabel')}
              value={formData.category}
              onChange={(e) => onFormDataChange({ ...formData, category: e.target.value })}
              options={[
                { value: t('chat.categoryTraining'), label: t('chat.categoryTraining') },
                { value: t('chat.categoryNutrition'), label: t('chat.categoryNutrition') },
                { value: t('chat.categoryRecovery'), label: t('chat.categoryRecovery') },
                { value: t('chat.categoryRecipes'), label: t('chat.categoryRecipes') },
                { value: t('chat.categoryMobility'), label: t('chat.categoryMobility') },
                { value: t('chat.categoryAnalysis'), label: t('chat.categoryAnalysis') },
                { value: t('chat.categoryCustom'), label: t('chat.categoryCustom') },
              ]}
            />

            {/* Data Context Section */}
            <div className="border-t ghost-border pt-4">
              <h3 className="text-sm font-medium text-on-surface-variant mb-3">{t('chat.dataContextSection')}</h3>

              <div className="mb-3">
                <Textarea
                  label={t('app.startupQueryOptional')}
                  placeholder={t('app.startupQueryPlaceholder')}
                  value={formData.startup_query}
                  onChange={(e) => onFormDataChange({ ...formData, startup_query: e.target.value })}
                  rows={2}
                />
              </div>

              <label className="flex items-center gap-2 mb-3 cursor-pointer">
                <input
                  type="checkbox"
                  checked={formData.prefetch_enabled}
                  onChange={(e) => onFormDataChange({ ...formData, prefetch_enabled: e.target.checked })}
                  className="w-4 h-4 rounded ghost-border text-primary focus:ring-primary"
                />
                <span className="text-sm text-on-surface-variant">{t('chat.prefetchActivityData')}</span>
              </label>

              {formData.prefetch_enabled && (
                <div className="space-y-3 pl-6 border-l-2 border-primary/20">
                  <div className="grid grid-cols-2 gap-3">
                    <div>
                      <label className="block text-xs font-medium text-on-surface-variant mb-1">{t('chat.activityCountLabel')}</label>
                      <input
                        type="number"
                        min={1}
                        max={200}
                        value={formData.activity_count}
                        onChange={(e) => onFormDataChange({ ...formData, activity_count: Math.max(1, Math.min(200, Number(e.target.value))) })}
                        className="w-full px-2 py-1.5 text-sm border ghost-border rounded-lg focus:outline-none focus:ring-2 focus:ring-primary focus:border-transparent"
                      />
                    </div>
                    <Select
                      label={t('chat.timeFrameLabel')}
                      size="sm"
                      value={formData.time_frame}
                      onChange={(e) => onFormDataChange({ ...formData, time_frame: e.target.value })}
                      options={[
                        { value: '3w', label: '3 weeks' },
                        { value: '8w', label: '8 weeks' },
                        { value: '12w', label: '12 weeks' },
                        { value: '16w', label: '16 weeks' },
                        { value: '6m', label: '6 months' },
                      ]}
                    />
                  </div>

                  <div className="flex items-center gap-4">
                    <Radio
                      name="detail_mode"
                      label={t('chat.detailLevelSummary')}
                      checked={formData.detail_mode === 'summary'}
                      onChange={() => onFormDataChange({ ...formData, detail_mode: 'summary' })}
                    />
                    <Radio
                      name="detail_mode"
                      label={t('app.detailedLapsSplits')}
                      checked={formData.detail_mode === 'detailed'}
                      onChange={() => onFormDataChange({ ...formData, detail_mode: 'detailed' })}
                    />
                  </div>

                  <label className="flex items-center gap-2 cursor-pointer">
                    <input
                      type="checkbox"
                      checked={formData.athlete_profile}
                      onChange={(e) => onFormDataChange({ ...formData, athlete_profile: e.target.checked })}
                      className="w-3.5 h-3.5 rounded ghost-border text-primary focus:ring-primary"
                    />
                    <span className="text-xs text-on-surface-variant">{t('chat.fetchAthleteProfile')}</span>
                  </label>
                </div>
              )}
            </div>

            {/* Tool Budget Section */}
            <div className="border-t ghost-border pt-4">
              <h3 className="text-sm font-medium text-on-surface-variant mb-3">{t('chat.toolBudgetSection')}</h3>
              <label
                htmlFor="max-tool-iterations"
                className="block text-xs font-medium text-on-surface-variant mb-1"
              >
                {t('chat.maxToolIterations')}
              </label>
              <input
                id="max-tool-iterations"
                type="number"
                min={MIN_MAX_TOOL_ITERATIONS}
                max={MAX_MAX_TOOL_ITERATIONS}
                value={formData.max_tool_iterations ?? ''}
                placeholder={String(DEFAULT_MAX_TOOL_ITERATIONS)}
                onChange={(e) =>
                  onFormDataChange({
                    ...formData,
                    max_tool_iterations: clampToolIterations(e.target.value),
                  })
                }
                className="w-full px-2 py-1.5 text-sm border ghost-border rounded-lg focus:outline-none focus:ring-2 focus:ring-primary focus:border-transparent"
              />
              <p className="mt-1 text-xs text-on-surface-variant">
                {t('frag.toolRoundsHelp')} {MIN_MAX_TOOL_ITERATIONS}–{MAX_MAX_TOOL_ITERATIONS}. Leave it empty to follow
                the workspace limit, currently {DEFAULT_MAX_TOOL_ITERATIONS}.
              </p>
            </div>

            <div className="flex gap-3 pt-2">
              <button
                type="button"
                onClick={onClose}
                className="flex-1 px-4 py-2 text-sm font-medium text-on-surface-variant bg-surface-container-high rounded-lg hover:bg-surface-container-highest transition-colors"
              >
                {t('chat.cancel')}
              </button>
              <button
                type="submit"
                disabled={isSubmitting || !formData.title.trim() || !formData.system_prompt.trim()}
                className="flex-1 px-4 py-2 text-sm font-medium text-on-primary bg-primary rounded-lg hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
              >
                {isSubmitting ? t('chat.saving') : t('chat.saveChanges')}
              </button>
            </div>

            {submitError && (
              <p className="text-xs text-error text-center">
                {t('discover.updateCoachFailed')}
              </p>
            )}

            {onDelete && (
              <div className="border-t ghost-border pt-3 text-center">
                <button
                  type="button"
                  onClick={onDelete}
                  disabled={isSubmitting}
                  className="text-xs font-medium text-error hover:underline disabled:opacity-50 min-h-[44px]"
                >
                  {t('discover.deleteThisCoach')}
                </button>
              </div>
            )}
          </form>
        </div>
      </div>
    </div>
  );
}
