// ABOUTME: The edit sheet for one of the athlete's own coaches — the only coach editor in the app
// ABOUTME: Single scrollable page with collapsible sections; saves through update, deletes the coach

import React, { useState, useEffect, useCallback } from 'react';
import {
  View,
  Text,
  ScrollView,
  TextInput,
  TouchableOpacity,
  ActivityIndicator,
  Alert,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter, useLocalSearchParams } from 'expo-router';
import * as Haptics from 'expo-haptics';
import { PRIMARY_PALETTE, spacing, useCardStyle, buttonGlow, useThemeColors, categoryAccent, categoryInk } from '../../constants/theme';
import { coachesApi } from '../../services/api';
import { CollapsibleSection } from '../../components/ui';
import type { UpdateCoachRequest } from '../../types';
import { useTranslation } from '@pierre/i18n';

// Category options with colors matching Stitch UX spec
// `key` is the value stored on the coach and sent to the API, so it stays
// English; `labelKey` is what the chip shows and is resolved at render.
// The colour is no longer carried here: it comes from `categoryAccent`, which
// reads the live pillar tokens, so this table holds only what is genuinely
// static about a category.
const CATEGORY_OPTIONS: Array<{ key: string; labelKey: string }> = [
  { key: 'training', labelKey: 'app.training' },
  { key: 'nutrition', labelKey: 'app.nutrition' },
  { key: 'recovery', labelKey: 'app.recovery' },
  { key: 'recipes', labelKey: 'app.recipes' },
  { key: 'mobility', labelKey: 'app.mobility' },
  { key: 'custom', labelKey: 'app.custom' },
];

// Validation constants
const MAX_TITLE_LENGTH = 100;
const MAX_DESCRIPTION_LENGTH = 500;
const MAX_SYSTEM_PROMPT_LENGTH = 8000;
const CONTEXT_WINDOW_SIZE = 128000;

export function CoachEditorScreen() {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const cardStyle = useCardStyle();
  const router = useRouter();
  const { coachId } = useLocalSearchParams<{ coachId: string }>();

  // Form state
  const [title, setTitle] = useState('');
  const [category, setCategory] = useState<string>('custom');
  const [description, setDescription] = useState('');
  const [systemPrompt, setSystemPrompt] = useState('');
  const [tags, setTags] = useState<string[]>([]);
  const [newTag, setNewTag] = useState('');

  // Data context state
  const [startupQuery, setStartupQuery] = useState('');
  const [prefetchEnabled, setPrefetchEnabled] = useState(false);
  const [activityCount, setActivityCount] = useState(20);
  const [timeFrame, setTimeFrame] = useState('12w');
  const [detailMode, setDetailMode] = useState<'summary' | 'detailed'>('summary');
  const [athleteProfile, setAthleteProfile] = useState(false);

  // UI state
  const [isLoading, setIsLoading] = useState(true);
  const [isSaving, setIsSaving] = useState(false);
  const [isDeleting, setIsDeleting] = useState(false);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [expandedTextArea, setExpandedTextArea] = useState(false);
  const [showCategoryModal, setShowCategoryModal] = useState(false);

  const loadCoach = useCallback(async (id: string) => {
    try {
      setIsLoading(true);
      const coach = await coachesApi.get(id);
      setTitle(coach.title);
      setCategory(coach.category);
      setDescription(coach.description || '');
      setSystemPrompt(coach.system_prompt);
      setTags(coach.tags || []);
      setStartupQuery(coach.startup_query || '');
      if (coach.data_requirements?.activities) {
        setPrefetchEnabled(true);
        setActivityCount(coach.data_requirements.activities.count);
        setTimeFrame(coach.data_requirements.activities.time_frame || '12w');
        setDetailMode(coach.data_requirements.activities.mode || 'summary');
        setAthleteProfile(coach.data_requirements?.athlete_profile || false);
      }
    } catch (error) {
      console.error('Failed to load coach:', error);
      Alert.alert(t('common.error'), t('app.failedLoadCoachData'));
      router.back();
    } finally {
      setIsLoading(false);
    }
  }, [router, t]);

  useEffect(() => {
    if (coachId) {
      loadCoach(coachId);
    }
  }, [coachId, loadCoach]);

  // Derived save-readiness for dynamic testID (Maestro sync point)
  const canSave = title.trim().length > 0 && systemPrompt.trim().length > 0 && !isSaving && !isDeleting;

  // Calculate token count (same formula as web)
  const tokenCount = Math.ceil(systemPrompt.length / 4);
  const contextPercentage = ((tokenCount / CONTEXT_WINDOW_SIZE) * 100).toFixed(1);

  // Validation
  const validate = useCallback((): boolean => {
    const newErrors: Record<string, string> = {};

    if (!title.trim()) {
      newErrors.title = t('validation.titleRequired');
    } else if (title.length > MAX_TITLE_LENGTH) {
      newErrors.title = t('app.maxLengthTitle', { max: MAX_TITLE_LENGTH });
    }

    if (description.length > MAX_DESCRIPTION_LENGTH) {
      newErrors.description = t('app.maxLengthDescription', { max: MAX_DESCRIPTION_LENGTH });
    }

    if (!systemPrompt.trim()) {
      newErrors.systemPrompt = t('validation.systemPromptRequired');
    } else if (systemPrompt.length > MAX_SYSTEM_PROMPT_LENGTH) {
      newErrors.systemPrompt = t('app.maxLengthSystemPrompt', { max: MAX_SYSTEM_PROMPT_LENGTH });
    }

    setErrors(newErrors);
    return Object.keys(newErrors).length === 0;
  }, [title, description, systemPrompt, t]);

  // Show category picker as a React Native Modal (cross-platform, Maestro-compatible)
  const showCategoryPicker = () => {
    setShowCategoryModal(true);
  };

  const selectCategory = (key: string) => {
    setCategory(key);
    setShowCategoryModal(false);
  };

  // Tag management
  const addTag = () => {
    const trimmedTag = newTag.trim().toLowerCase();
    if (trimmedTag && !tags.includes(trimmedTag) && tags.length < 10) {
      setTags([...tags, trimmedTag]);
      setNewTag('');
    }
  };

  const removeTag = (tagToRemove: string) => {
    setTags(tags.filter((tag) => tag !== tagToRemove));
  };

  // Save handler
  const handleSave = async () => {
    if (!coachId || !validate()) return;

    try {
      setIsSaving(true);
      Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);

      // Build data_requirements from structured fields
      const dataRequirements = prefetchEnabled
        ? {
            activities: {
              count: activityCount,
              time_frame: timeFrame,
              mode: detailMode,
              format: 'toon' as const,
              analysis_type: 'general_overview',
            },
            athlete_profile: athleteProfile,
          }
        : undefined;

      const updateData: UpdateCoachRequest = {
        title: title.trim(),
        category,
        description: description.trim() || undefined,
        system_prompt: systemPrompt.trim(),
        tags,
        startup_query: startupQuery.trim() || undefined,
        data_requirements: dataRequirements,
      };
      await coachesApi.update(coachId, updateData);

      Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
      router.back();
    } catch (error) {
      console.error('Failed to save coach:', error);
      Alert.alert(t('common.error'), t('app.failedUpdateCoach'));
    } finally {
      setIsSaving(false);
    }
  };

  // Delete handler: confirmation, then the coach is gone and so is this screen.
  const handleDelete = () => {
    if (!coachId) return;
    Alert.alert(
      t('app.deleteCoachQ'),
      t('app.confirmDeleteCoach', { coach: title }),
      [
        { text: t('common.cancel'), style: 'cancel' },
        {
          text: t('common.delete'),
          style: 'destructive',
          onPress: async () => {
            try {
              setIsDeleting(true);
              await coachesApi.delete(coachId);
              Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
              router.back();
            } catch (error) {
              console.error('Failed to delete coach:', error);
              Alert.alert(t('common.error'), t('app.failedDeleteCoach'));
            } finally {
              setIsDeleting(false);
            }
          },
        },
      ],
    );
  };

  // Get current category info
  const currentCategory = CATEGORY_OPTIONS.find((c) => c.key === category);

  if (!coachId) {
    return (
      <SafeAreaView className="flex-1 bg-background-primary" testID="coach-editor-missing">
        <View className="flex-1 justify-center items-center p-6">
          <Text className="text-lg text-text-secondary mb-3">{t('app.coachNotFound')}</Text>
          <TouchableOpacity
            className="px-5 py-2 bg-primary-500 rounded-lg"
            onPress={() => router.back()}
            testID="back-button"
          >
            <Text className="text-text-primary text-base font-medium">{t('app.goBack')}</Text>
          </TouchableOpacity>
        </View>
      </SafeAreaView>
    );
  }

  if (isLoading) {
    return (
      <SafeAreaView className="flex-1 bg-background-primary">
        <View className="flex-1 items-center justify-center">
          <ActivityIndicator size="large" color={PRIMARY_PALETTE[500]} />
          <Text className="text-text-secondary mt-3 text-base">{t('app.loadingCoach')}</Text>
        </View>
      </SafeAreaView>
    );
  }

  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="coach-editor-screen">
      <KeyboardAvoidingView
        className="flex-1"
        behavior={Platform.OS === 'ios' ? 'padding' : undefined}
      >
        {/* Header */}
        <View className="flex-row items-center px-3 py-2 border-b border-border-subtle">
          <TouchableOpacity
            className="w-10 h-10 items-center justify-center"
            onPress={() => router.back()}
            testID="back-button"
          >
            <Text className="text-2xl text-text-primary">{'←'}</Text>
          </TouchableOpacity>
          <Text className="flex-1 text-lg font-semibold text-text-primary text-center">
            {t('app.editCoachTitle')}
          </Text>
          <TouchableOpacity
            className={`px-4 py-1.5 rounded-xl min-w-[60px] items-center ${isSaving ? 'opacity-60' : ''}`}
            style={{
              backgroundColor: colors.pierre.violet,
              ...buttonGlow,
            }}
            onPress={handleSave}
            disabled={isSaving || isDeleting}
            testID={canSave ? 'save-button' : 'save-button-disabled'}
          >
            {isSaving ? (
              <ActivityIndicator size="small" color={colors.tokens.onPrimary} />
            ) : (
              <Text className="text-base font-semibold" style={{ color: colors.tokens.onPrimary }}>{t('common.save')}</Text>
            )}
          </TouchableOpacity>
        </View>

        <ScrollView
          className="flex-1"
          contentContainerStyle={{ padding: spacing.lg, paddingBottom: spacing.xxl }}
          keyboardShouldPersistTaps="handled"
        >
          {/* Title Field */}
          <View className="mb-5">
            <Text className="text-text-primary text-sm font-semibold mb-2">{t('app.titleRequired')}</Text>
            <TextInput
              testID="coach-title-input"
              className="p-3.5 text-text-primary text-base"
              style={{
                ...cardStyle,
                borderRadius: 12,
                borderColor: errors.title ? colors.error : colors.border.default,
              }}
              value={title}
              onChangeText={setTitle}
              placeholder={t('app.enterCoachTitle')}
              placeholderTextColor={colors.text.tertiary}
              maxLength={MAX_TITLE_LENGTH}
            />
            <Text className="text-text-tertiary text-xs text-right mt-1">
              {title.length}/{MAX_TITLE_LENGTH}
            </Text>
            {errors.title && (
              <Text className="text-error text-xs mt-1" testID="title-error">
                {errors.title}
              </Text>
            )}
          </View>

          {/* Category Field */}
          <View className="mb-5">
            <Text className="text-text-primary text-sm font-semibold mb-2">{t('app.category')}</Text>
            <TouchableOpacity
              className="flex-row items-center justify-between p-3.5"
              style={{
                ...cardStyle,
                borderRadius: 12,
              }}
              onPress={showCategoryPicker}
              testID="category-picker"
            >
              {/* The selected category reads as a TINT of its pillar accent,
                  labelled in that accent's bound ink. A pillar hue is a fill,
                  never a ground for `on-surface`: the pale dark-scheme set
                  under near-white ink measures 1.32-2.11:1, the pairing
                  DESIGN.md §5 lists under Forbidden. `categoryAccent` at /20
                  with `categoryInk` on top clears AA for every category in
                  both schemes. Both read `category`, so an unknown one takes
                  the primary/on-primary-container pair together. */}
              <View
                className="px-3 py-1.5 rounded-full"
                style={{ backgroundColor: `${categoryAccent(colors, category)}20` }}
                testID="selected-category"
              >
                <Text className="text-sm font-semibold" style={{ color: categoryInk(colors, category) }}>
                  {currentCategory ? t(currentCategory.labelKey) : undefined}
                </Text>
              </View>
              <Text className="text-text-secondary text-sm">{'▼'}</Text>
            </TouchableOpacity>
          </View>

          {/* Description Field */}
          <View className="mb-5">
            <Text className="text-text-primary text-sm font-semibold mb-2">{t('app.description')}</Text>
            <TextInput
              testID="coach-description-input"
              className="p-3.5 text-text-primary text-base min-h-[100px]"
              style={{
                ...cardStyle,
                borderRadius: 12,
                borderColor: errors.description ? colors.error : colors.border.default,
              }}
              value={description}
              onChangeText={setDescription}
              placeholder={t('app.describeCoachPlaceholder')}
              placeholderTextColor={colors.text.tertiary}
              multiline
              numberOfLines={3}
              maxLength={MAX_DESCRIPTION_LENGTH}
              textAlignVertical="top"
            />
            <Text className="text-text-tertiary text-xs text-right mt-1">
              {description.length}/{MAX_DESCRIPTION_LENGTH}
            </Text>
            {errors.description && (
              <Text className="text-error text-xs mt-1" testID="description-error">
                {errors.description}
              </Text>
            )}
          </View>

          {/* System Prompt Section (collapsible, expanded by default) */}
          <CollapsibleSection
            title={t('app.systemPromptRequired')}
            defaultExpanded
            testID="system-prompt-section"
          >
            <View className="flex-row justify-end mb-2">
              <TouchableOpacity
                onPress={() => setExpandedTextArea(true)}
                testID="expand-prompt-button"
              >
                <Text style={{ color: colors.pierre.violet }} className="text-sm">
                  {t('app.expand')} {'↗'}
                </Text>
              </TouchableOpacity>
            </View>
            <TextInput
              testID="system-prompt-input"
              className="p-3.5 text-text-primary text-base min-h-[200px]"
              style={{
                ...cardStyle,
                borderRadius: 12,
                borderColor: errors.systemPrompt ? colors.error : colors.border.default,
              }}
              value={systemPrompt}
              onChangeText={setSystemPrompt}
              placeholder={t('app.definePromptPlaceholderLong')}
              placeholderTextColor={colors.text.tertiary}
              multiline
              textAlignVertical="top"
            />
            {errors.systemPrompt && (
              <Text className="text-error text-xs mt-1" testID="prompt-error">
                {errors.systemPrompt}
              </Text>
            )}

            {/* Token counter with gradient progress bar */}
            <View
              className="mt-3 p-3 rounded-xl"
              style={{ ...cardStyle, borderRadius: 12 }}
              testID="token-counter"
            >
              <Text className="text-text-secondary text-sm mb-2" testID="token-count-text">
                ~{tokenCount.toLocaleString()} tokens ({contextPercentage}% of context)
              </Text>
              <View
                className="h-1.5 rounded-full overflow-hidden"
                style={{ backgroundColor: colors.background.tertiary }}
              >
                {/* A meter reads its value from length, not from a colour
                    ramp, and the ramp here was a fixed pair that ignored the
                    scheme. One solid `primary` fill says the same thing in
                    both. */}
                <View
                  style={{
                    height: '100%',
                    width: `${Math.min(parseFloat(contextPercentage), 100)}%`,
                    borderRadius: 3,
                    backgroundColor: colors.tokens.primary,
                  }}
                />
              </View>
            </View>
          </CollapsibleSection>

          {/* Tags Section (collapsible, collapsed by default) */}
          <CollapsibleSection title={t('app.tags')} defaultExpanded={false} testID="tags-section">
            <View className="flex-row gap-2">
              <TextInput
                testID="tag-input"
                className="flex-1 p-3.5 text-text-primary text-base"
                style={{
                  ...cardStyle,
                  borderRadius: 12,
                }}
                value={newTag}
                onChangeText={setNewTag}
                placeholder={t('app.addATag')}
                placeholderTextColor={colors.text.tertiary}
                onSubmitEditing={addTag}
                returnKeyType="done"
              />
              <TouchableOpacity
                className="w-12 justify-center items-center rounded-xl"
                style={{
                  backgroundColor: colors.pierre.violet,
                  ...buttonGlow,
                }}
                onPress={addTag}
                testID="add-tag-button"
              >
                <Text className="text-xl font-bold" style={{ color: colors.tokens.onPrimary }}>+</Text>
              </TouchableOpacity>
            </View>
            <View className="flex-row flex-wrap gap-2 mt-3" testID="tags-container">
              {tags.map((tag) => (
                <View
                  key={tag}
                  className="flex-row items-center px-3 py-1.5 rounded-full gap-1"
                  style={{
                    backgroundColor: colors.background.tertiary,
                    borderWidth: 1,
                    borderColor: colors.border.default,
                  }}
                  testID={`tag-chip-${tag}`}
                >
                  <Text style={{ color: colors.pierre.violet }} className="text-sm">
                    {tag}
                  </Text>
                  <TouchableOpacity
                    onPress={() => removeTag(tag)}
                    hitSlop={{ top: 10, bottom: 10, left: 10, right: 10 }}
                    testID={`remove-tag-${tag}`}
                  >
                    <Text style={{ color: colors.pierre.violet }} className="text-lg font-bold">
                      {'×'}
                    </Text>
                  </TouchableOpacity>
                </View>
              ))}
              {tags.length === 0 && (
                <Text className="text-text-tertiary text-sm italic" testID="no-tags-message">
                  {t('app.noTagsYet')}
                </Text>
              )}
            </View>
          </CollapsibleSection>

          {/* Data Context Section (collapsible) */}
          <CollapsibleSection title={t('app.dataContext')} defaultExpanded={false} testID="data-context-section">
            <View className="mb-4">
              <Text className="text-text-primary text-sm font-semibold mb-2">{t('app.startupQuery')}</Text>
              <TextInput
                testID="startup-query-input"
                className="p-3.5 text-text-primary text-base min-h-[80px]"
                style={{
                  ...cardStyle,
                  borderRadius: 12,
                }}
                value={startupQuery}
                onChangeText={setStartupQuery}
                placeholder={t('app.whatAnalyzeFirst')}
                placeholderTextColor={colors.text.tertiary}
                multiline
                textAlignVertical="top"
              />
            </View>

            <TouchableOpacity
              className="flex-row items-center mb-4"
              onPress={() => setPrefetchEnabled(!prefetchEnabled)}
              testID="prefetch-toggle"
            >
              <View
                className="w-5 h-5 rounded mr-3 items-center justify-center"
                style={{
                  backgroundColor: prefetchEnabled ? colors.pierre.violet : 'transparent',
                  borderWidth: prefetchEnabled ? 0 : 1.5,
                  borderColor: colors.border.strong,
                }}
              >
                {prefetchEnabled && (
                  <Text className="text-xs font-bold" style={{ color: colors.tokens.onPrimary }}>{'✓'}</Text>
                )}
              </View>
              <Text className="text-text-primary text-sm">{t('app.prefetchActivity')}</Text>
            </TouchableOpacity>

            {prefetchEnabled && (
              <View className="pl-3" style={{ borderLeftWidth: 2, borderLeftColor: colors.border.default }}>
                <View className="flex-row gap-3 mb-3">
                  <View className="flex-1">
                    <Text className="text-text-secondary text-xs font-semibold mb-1">{t('app.activityCount')}</Text>
                    <TextInput
                      testID="activity-count-input"
                      className="p-2.5 text-text-primary text-sm"
                      style={{ ...cardStyle, borderRadius: 10 }}
                      value={String(activityCount)}
                      onChangeText={(v) => setActivityCount(Math.max(1, Math.min(200, Number(v) || 1)))}
                      keyboardType="number-pad"
                    />
                  </View>
                  <View className="flex-1">
                    <Text className="text-text-secondary text-xs font-semibold mb-1">{t('app.timeFrame')}</Text>
                    <TouchableOpacity
                      className="p-2.5 flex-row items-center justify-between"
                      style={{ ...cardStyle, borderRadius: 10 }}
                      onPress={() => {
                        const frames = ['3w', '8w', '12w', '16w', '6m'];
                        const idx = frames.indexOf(timeFrame);
                        setTimeFrame(frames[(idx + 1) % frames.length]);
                      }}
                      testID="time-frame-picker"
                    >
                      <Text className="text-text-primary text-sm">
                        {timeFrame === '3w' ? '3 weeks' : timeFrame === '8w' ? '8 weeks' : timeFrame === '12w' ? '12 weeks' : timeFrame === '16w' ? '16 weeks' : '6 months'}
                      </Text>
                      <Text className="text-text-tertiary text-xs">{'▼'}</Text>
                    </TouchableOpacity>
                  </View>
                </View>

                <View className="flex-row gap-4 mb-3">
                  <TouchableOpacity
                    className="flex-row items-center"
                    onPress={() => setDetailMode('summary')}
                  >
                    <View
                      className="w-4 h-4 rounded-full mr-2 items-center justify-center"
                      style={{
                        borderWidth: 1.5,
                        borderColor: colors.pierre.violet,
                        backgroundColor: detailMode === 'summary' ? colors.pierre.violet : 'transparent',
                      }}
                    />
                    <Text className="text-text-secondary text-xs">{t('app.summary')}</Text>
                  </TouchableOpacity>
                  <TouchableOpacity
                    className="flex-row items-center"
                    onPress={() => setDetailMode('detailed')}
                  >
                    <View
                      className="w-4 h-4 rounded-full mr-2 items-center justify-center"
                      style={{
                        borderWidth: 1.5,
                        borderColor: colors.pierre.violet,
                        backgroundColor: detailMode === 'detailed' ? colors.pierre.violet : 'transparent',
                      }}
                    />
                    <Text className="text-text-secondary text-xs">{t('app.detailedLapsSplits')}</Text>
                  </TouchableOpacity>
                </View>

                <TouchableOpacity
                  className="flex-row items-center"
                  onPress={() => setAthleteProfile(!athleteProfile)}
                >
                  <View
                    className="w-4 h-4 rounded mr-2 items-center justify-center"
                    style={{
                      backgroundColor: athleteProfile ? colors.pierre.violet : 'transparent',
                      borderWidth: athleteProfile ? 0 : 1.5,
                      borderColor: colors.border.strong,
                    }}
                  >
                    {athleteProfile && (
                      <Text className="text-[10px] font-bold" style={{ color: colors.tokens.onPrimary }}>{'✓'}</Text>
                    )}
                  </View>
                  <Text className="text-text-secondary text-xs">{t('app.alsoFetchProfile')}</Text>
                </TouchableOpacity>
              </View>
            )}
          </CollapsibleSection>

          {/* Delete: the coach leaves the athlete's list, and this sheet closes with it */}
          <TouchableOpacity
            className="mt-6 py-3.5 rounded-xl items-center border"
            style={{ borderColor: colors.error }}
            onPress={handleDelete}
            disabled={isSaving || isDeleting}
            testID="delete-coach-button"
          >
            {isDeleting ? (
              <ActivityIndicator size="small" color={colors.error} />
            ) : (
              <Text className="text-base font-semibold" style={{ color: colors.error }}>{t('app.deleteCoach')}</Text>
            )}
          </TouchableOpacity>
        </ScrollView>
      </KeyboardAvoidingView>

      {/* Fullscreen System Prompt Modal */}
      <Modal
        visible={expandedTextArea}
        animationType="slide"
        presentationStyle="pageSheet"
      >
        <SafeAreaView className="flex-1 bg-background-primary" testID="expanded-modal">
          <View className="flex-row items-center justify-between px-3 py-2 border-b border-border-default">
            <TouchableOpacity
              onPress={() => setExpandedTextArea(false)}
              testID="modal-done-button"
            >
              <Text className="text-primary-500 text-base font-semibold">{t('app.done')}</Text>
            </TouchableOpacity>
            <Text className="text-text-primary text-base font-semibold">{t('app.systemPrompt')}</Text>
            <View className="w-[50px]" />
          </View>
          <TextInput
            testID="modal-text-input"
            className="flex-1 p-3 text-text-primary text-base"
            value={systemPrompt}
            onChangeText={setSystemPrompt}
            placeholder={t('app.definePromptPlaceholder')}
            placeholderTextColor={colors.text.tertiary}
            multiline
            textAlignVertical="top"
            autoFocus
          />
          <View className="px-3 py-2 border-t border-border-default">
            <Text className="text-text-secondary text-sm" testID="modal-token-count">
              ~{tokenCount.toLocaleString()} tokens ({contextPercentage}% of context)
            </Text>
          </View>
        </SafeAreaView>
      </Modal>

      {/* Category Picker Modal */}
      <Modal
        visible={showCategoryModal}
        transparent
        animationType="fade"
        onRequestClose={() => setShowCategoryModal(false)}
      >
        <Pressable
          className="flex-1 justify-end"
          style={{ backgroundColor: 'rgba(0,0,0,0.5)' }}
          onPress={() => setShowCategoryModal(false)}
        >
          {/* The sheet sits over the page, so it is a raised surface and takes
              the card recipe: the scheme-correct lifted fill plus a hairline,
              no shadow. Its title and options are `text-text-primary`, which
              is on-surface — ink that only reads against a ground the same
              scheme chose. */}
          <Pressable
            className="rounded-t-2xl p-5 pb-10"
            style={cardStyle}
            onPress={() => {/* prevent dismiss when tapping content */}}
          >
            <Text className="text-text-primary text-lg font-bold text-center mb-4">
              {t('app.selectCategory')}
            </Text>
            {CATEGORY_OPTIONS.map((cat) => (
              <TouchableOpacity
                key={cat.key}
                className="flex-row items-center py-3.5 px-4 mb-1 rounded-xl"
                style={{
                  backgroundColor: category === cat.key ? colors.background.tertiary : 'transparent',
                }}
                onPress={() => selectCategory(cat.key)}
                testID={`category-option-${cat.key}`}
              >
                <View
                  className="w-3 h-3 rounded-full mr-3"
                  style={{ backgroundColor: categoryAccent(colors, cat.key) }}
                />
                <Text className="text-text-primary text-base">{t(cat.labelKey)}</Text>
              </TouchableOpacity>
            ))}
            <TouchableOpacity
              className="mt-3 py-3.5 rounded-xl"
              style={{ backgroundColor: colors.background.tertiary }}
              onPress={() => setShowCategoryModal(false)}
              testID="category-cancel"
            >
              <Text className="text-text-secondary text-base text-center font-semibold">
                {t('common.cancel')}
              </Text>
            </TouchableOpacity>
          </Pressable>
        </Pressable>
      </Modal>
    </SafeAreaView>
  );
}
