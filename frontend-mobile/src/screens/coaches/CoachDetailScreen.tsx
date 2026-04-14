// ABOUTME: Coach detail screen showing full coach info with edit/delete actions
// ABOUTME: Read-only view of user's coach with option to edit or use in chat

import React, { useState, useCallback, useEffect } from 'react';
import {
  View,
  Text,

  ScrollView,
  TouchableOpacity,
  ActivityIndicator,
  Alert,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter, useLocalSearchParams } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import { LinearGradient } from 'expo-linear-gradient';
import Markdown from 'react-native-markdown-display';
import { Platform } from 'react-native';
import { colors, spacing, fontSize, borderRadius, glassCard, gradients, buttonGlow } from '../../constants/theme';
import { coachesApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { TAB_BAR_BOTTOM_OFFSET } from '../../components/ui/ExpandableTabBar';
import type { Coach } from '../../types';

const coachMarkdownStyles = {
  body: { color: colors.text.secondary, fontSize: fontSize.md, lineHeight: fontSize.md * 1.6 },
  heading2: { color: colors.text.primary, fontSize: fontSize.lg, fontWeight: '600' as const, marginTop: spacing.sm, marginBottom: spacing.xs },
  heading3: { color: colors.text.primary, fontSize: fontSize.md, fontWeight: '600' as const, marginTop: spacing.xs },
  strong: { color: colors.text.primary, fontWeight: '700' as const },
  em: { color: colors.text.secondary, fontStyle: 'italic' as const },
  bullet_list: { marginLeft: spacing.sm },
  ordered_list: { marginLeft: spacing.sm },
  list_item: { marginBottom: spacing.xs },
  code_inline: { backgroundColor: colors.background.tertiary, color: colors.primary[400], paddingHorizontal: 4, borderRadius: 4, fontFamily: Platform.OS === 'ios' ? 'Menlo' : 'monospace', fontSize: fontSize.sm },
  fence: { backgroundColor: colors.background.tertiary, borderRadius: borderRadius.sm, padding: spacing.sm, marginVertical: spacing.xs },
  link: { color: colors.primary[400], textDecorationLine: 'underline' as const },
  hr: { backgroundColor: colors.border.default, height: 1, marginVertical: spacing.sm },
};

// Coach category colors matching Stitch UX spec
const COACH_CATEGORY_COLORS: Record<string, string> = {
  training: '#3c6658',  // Green per Stitch spec
  nutrition: '#8f6a2e', // Amber per Stitch spec
  recovery: '#0d3b2e',  // Cyan per Stitch spec
  recipes: '#8f6a2e',   // Amber
  mobility: '#7a4d5e',  // Pink - for stretching/yoga
  custom: '#00241a',    // Violet per Stitch spec
};

export function CoachDetailScreen() {
  const router = useRouter();
  const { coachId } = useLocalSearchParams<{ coachId: string }>();
  const { isAuthenticated } = useAuth();
  const [coach, setCoach] = useState<Coach | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isDeleting, setIsDeleting] = useState(false);
  const [isHidden, setIsHidden] = useState(false);
  const [isTogglingHidden, setIsTogglingHidden] = useState(false);

  const loadCoachDetail = useCallback(async () => {
    if (!isAuthenticated || !coachId) return;

    try {
      setIsLoading(true);
      // Load coaches and hidden coaches list in parallel
      const [coachesResponse, hiddenResponse] = await Promise.all([
        coachesApi.listCoaches({ include_hidden: true }),
        coachesApi.getHiddenCoaches(),
      ]);
      const foundCoach = coachesResponse.coaches.find((c: { id: string }) => c.id === coachId);
      setCoach(foundCoach || null);

      // Check if this coach is in the hidden list
      const hiddenIds = new Set((hiddenResponse.coaches || []).map((c: { id: string }) => c.id));
      setIsHidden(hiddenIds.has(coachId));
    } catch (error) {
      console.error('Failed to load coach detail:', error);
      Alert.alert('Error', 'Failed to load coach details');
    } finally {
      setIsLoading(false);
    }
  }, [isAuthenticated, coachId]);

  useEffect(() => {
    loadCoachDetail();
  }, [loadCoachDetail]);

  const handleEdit = () => {
    if (!coach) return;
    router.push({ pathname: '/(app)/(tabs)/(coaches)/editor', params: { coachId: coach.id } });
  };

  const handleDelete = () => {
    if (!coach) return;

    Alert.alert(
      'Delete Coach?',
      `Are you sure you want to delete "${coach.title}"? This action cannot be undone.`,
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Delete',
          style: 'destructive',
          onPress: async () => {
            try {
              setIsDeleting(true);
              await coachesApi.deleteCoach(coach.id);
              Alert.alert('Deleted', 'Coach has been deleted.');
              router.back();
            } catch (error) {
              console.error('Failed to delete coach:', error);
              Alert.alert('Error', 'Failed to delete coach. Please try again.');
            } finally {
              setIsDeleting(false);
            }
          },
        },
      ]
    );
  };

  const handleUseInChat = () => {
    if (!coach) return;
    router.push('/(app)/(tabs)/(chat)');
  };

  const handleToggleHidden = async () => {
    if (!coach) return;

    try {
      setIsTogglingHidden(true);
      if (isHidden) {
        await coachesApi.show(coach.id);
        setIsHidden(false);
      } else {
        await coachesApi.hide(coach.id);
        setIsHidden(true);
      }
    } catch (error) {
      console.error('Failed to toggle coach visibility:', error);
      Alert.alert('Error', 'Failed to update coach visibility');
    } finally {
      setIsTogglingHidden(false);
    }
  };

  if (isLoading) {
    return (
      <SafeAreaView className="flex-1 bg-background-primary">
        <View className="flex-1 justify-center items-center">
          <ActivityIndicator size="large" color={colors.primary[500]} />
          <Text className="mt-3 text-text-secondary text-base">Loading coach details...</Text>
        </View>
      </SafeAreaView>
    );
  }

  if (!coach) {
    return (
      <SafeAreaView className="flex-1 bg-background-primary">
        <View className="flex-1 justify-center items-center px-6">
          <Text className="text-lg text-text-secondary mb-3">Coach not found</Text>
          <TouchableOpacity
            className="px-5 py-2 bg-primary-500 rounded-md"
            onPress={() => router.back()}
          >
            <Text className="text-text-primary text-base font-medium">Go Back</Text>
          </TouchableOpacity>
        </View>
      </SafeAreaView>
    );
  }

  const categoryColor = COACH_CATEGORY_COLORS[coach.category];

  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="coach-detail-screen">
      {/* Header */}
      <View className="flex-row items-center px-3 py-2 border-b border-border-default">
        <TouchableOpacity
          testID="back-button"
          className="p-2"
          onPress={() => router.back()}
        >
          <Text className="text-2xl text-text-primary">←</Text>
        </TouchableOpacity>
        <Text className="flex-1 text-lg font-semibold text-text-primary text-center mx-2" numberOfLines={1}>
          {coach.title}
        </Text>
        {!coach.is_system && (
          <TouchableOpacity
            testID="edit-button"
            className="p-2"
            onPress={handleEdit}
          >
            <Feather name="edit-2" size={20} color={colors.primary[500]} />
          </TouchableOpacity>
        )}
        {coach.is_system && <View className="w-10" />}
      </View>

      <ScrollView className="flex-1" showsVerticalScrollIndicator={false}>
        {/* Category & Stats */}
        <View className="flex-row justify-between items-center px-5 pt-5 pb-2">
          <View className="flex-row items-center gap-2">
            <View
              testID="category-badge"
              className="px-3 py-1 rounded-full"
              style={{ backgroundColor: categoryColor + '20' }}
            >
              <Text className="text-sm font-semibold capitalize" style={{ color: categoryColor }}>
                {coach.category}
              </Text>
            </View>
            {coach.is_system && (
              <View className="px-2 py-1 rounded" style={{ backgroundColor: colors.primary[500] + '30' }}>
                <Text className="text-xs font-semibold text-primary-500">System</Text>
              </View>
            )}
            {coach.is_favorite && (
              <Feather name="star" size={16} color="#8f6a2e" style={{ marginLeft: spacing.xs }} />
            )}
          </View>
          <Text testID="use-count" className="text-sm text-text-secondary">
            Used {coach.use_count} {coach.use_count === 1 ? 'time' : 'times'}
          </Text>
        </View>

        {/* Title */}
        <Text testID="coach-title" className="text-2xl font-bold text-text-primary px-5 mb-2">{coach.title}</Text>

        {/* Description */}
        {coach.description && (
          <View className="px-5 mb-3">
            <Markdown style={coachMarkdownStyles}>{coach.description}</Markdown>
          </View>
        )}

        {/* Tags */}
        {coach.tags.length > 0 && (
          <View className="px-5 py-3">
            <Text className="text-sm font-semibold text-text-secondary uppercase tracking-wide mb-2">Tags</Text>
            <View className="flex-row flex-wrap">
              {coach.tags.map((tag) => (
                <View
                  key={tag}
                  className="px-3 py-1.5 rounded-full mr-2 mb-2"
                  style={{
                    backgroundColor: 'rgba(0, 36, 26, 0.15)',
                    borderWidth: 1,
                    borderColor: 'rgba(0, 36, 26, 0.3)',
                  }}
                >
                  <Text className="text-sm" style={{ color: colors.pierre.violet }}>{tag}</Text>
                </View>
              ))}
            </View>
          </View>
        )}

        {/* Structured Sections (when available) or flat System Prompt */}
        {coach.purpose || coach.instructions ? (
          <>
            {coach.purpose && (
              <View className="px-5 py-3">
                <Text className="text-sm font-semibold text-text-secondary uppercase tracking-wide mb-2">Purpose</Text>
                <View style={{ ...glassCard, borderRadius: 12, overflow: 'hidden' }}>
                  <LinearGradient
                    colors={[categoryColor, `${categoryColor}40`] as [string, string]}
                    start={{ x: 0, y: 0 }}
                    end={{ x: 1, y: 0 }}
                    style={{ height: 2, width: '100%' }}
                  />
                  <View className="p-4">
                    <Markdown style={coachMarkdownStyles}>{coach.purpose}</Markdown>
                  </View>
                </View>
              </View>
            )}
            {coach.when_to_use && (
              <View className="px-5 py-3">
                <Text className="text-sm font-semibold text-text-secondary uppercase tracking-wide mb-2">When to Use</Text>
                <View style={{ ...glassCard, borderRadius: 12, overflow: 'hidden' }}>
                  <View className="p-4">
                    <Markdown style={coachMarkdownStyles}>{coach.when_to_use}</Markdown>
                  </View>
                </View>
              </View>
            )}
            {coach.instructions && (
              <View className="px-5 py-3">
                <Text className="text-sm font-semibold text-text-secondary uppercase tracking-wide mb-2">Instructions</Text>
                <View style={{ ...glassCard, borderRadius: 12, overflow: 'hidden' }}>
                  <LinearGradient
                    colors={[categoryColor, `${categoryColor}40`] as [string, string]}
                    start={{ x: 0, y: 0 }}
                    end={{ x: 1, y: 0 }}
                    style={{ height: 2, width: '100%' }}
                  />
                  <View className="p-4">
                    <Markdown style={coachMarkdownStyles}>{coach.instructions}</Markdown>
                  </View>
                </View>
              </View>
            )}
            {coach.example_inputs && (
              <View className="px-5 py-3">
                <Text className="text-sm font-semibold text-text-secondary uppercase tracking-wide mb-2">Example Inputs</Text>
                <View style={{ ...glassCard, borderRadius: 12, overflow: 'hidden' }}>
                  <View className="p-4">
                    <Markdown style={coachMarkdownStyles}>{coach.example_inputs}</Markdown>
                  </View>
                </View>
              </View>
            )}
            {coach.example_outputs && (
              <View className="px-5 py-3">
                <Text className="text-sm font-semibold text-text-secondary uppercase tracking-wide mb-2">Example Outputs</Text>
                <View style={{ ...glassCard, borderRadius: 12, overflow: 'hidden' }}>
                  <View className="p-4">
                    <Markdown style={coachMarkdownStyles}>{coach.example_outputs}</Markdown>
                  </View>
                </View>
              </View>
            )}
            {coach.success_criteria && (
              <View className="px-5 py-3">
                <Text className="text-sm font-semibold text-text-secondary uppercase tracking-wide mb-2">Success Criteria</Text>
                <View style={{ ...glassCard, borderRadius: 12, overflow: 'hidden' }}>
                  <View className="p-4">
                    <Markdown style={coachMarkdownStyles}>{coach.success_criteria}</Markdown>
                  </View>
                </View>
              </View>
            )}
          </>
        ) : (
          <View className="px-5 py-3">
            <Text className="text-sm font-semibold text-text-secondary uppercase tracking-wide mb-2">System Prompt</Text>
            <View style={{ ...glassCard, borderRadius: 12, overflow: 'hidden' }}>
              <LinearGradient
                colors={[categoryColor, `${categoryColor}40`] as [string, string]}
                start={{ x: 0, y: 0 }}
                end={{ x: 1, y: 0 }}
                style={{ height: 2, width: '100%' }}
              />
              <View className="p-4">
                <Markdown style={coachMarkdownStyles}>{coach.system_prompt}</Markdown>
              </View>
            </View>
          </View>
        )}

        {/* Metadata */}
        <View className="px-5 py-3">
          <Text className="text-sm font-semibold text-text-secondary uppercase tracking-wide mb-2">Details</Text>
          <View style={{ ...glassCard, borderRadius: 12, overflow: 'hidden' }}>
            <LinearGradient
              colors={gradients.violetCyan as [string, string]}
              start={{ x: 0, y: 0 }}
              end={{ x: 1, y: 0 }}
              style={{ height: 2, width: '100%' }}
            />
            <View className="flex-row justify-between items-center px-4 py-3 border-b ghost-border">
              <Text className="text-sm text-text-secondary">Token Count</Text>
              <Text className="text-sm text-text-primary font-medium">{coach.token_count}</Text>
            </View>
            <View className="flex-row justify-between items-center px-4 py-3 border-b ghost-border">
              <Text className="text-sm text-text-secondary">Context Usage</Text>
              <Text className="text-sm text-text-primary font-medium">
                {((coach.token_count / 128000) * 100).toFixed(1)}%
              </Text>
            </View>
            {coach.created_at && (
              <View className="flex-row justify-between items-center px-4 py-3 border-b ghost-border">
                <Text className="text-sm text-text-secondary">Created</Text>
                <Text className="text-sm text-text-primary font-medium">
                  {new Date(coach.created_at).toLocaleDateString()}
                </Text>
              </View>
            )}
            {coach.last_used_at && (
              <View className="flex-row justify-between items-center px-4 py-3">
                <Text className="text-sm text-text-secondary">Last Used</Text>
                <Text className="text-sm text-text-primary font-medium">
                  {new Date(coach.last_used_at).toLocaleDateString()}
                </Text>
              </View>
            )}
          </View>
        </View>

        {/* Bottom Spacer for Action Buttons + Tab Bar */}
        <View style={{ height: TAB_BAR_BOTTOM_OFFSET + 80 }} />
      </ScrollView>

      {/* Action Bar - Fixed above floating tab bar */}
      <View
        className="absolute left-0 right-0 flex-row p-4 pb-3 gap-3"
        style={{
          bottom: TAB_BAR_BOTTOM_OFFSET,
          backgroundColor: 'rgba(15, 15, 23, 0.95)',
          borderTopWidth: 1,
          borderTopColor: 'rgba(0, 36, 26, 0.2)',
        }}
      >
        <TouchableOpacity
          className="flex-1 flex-row items-center justify-center py-3.5 rounded-xl gap-2"
          style={{
            backgroundColor: colors.pierre.violet,
            ...buttonGlow,
          }}
          onPress={handleUseInChat}
          testID="use-in-chat-button"
        >
          <Feather name="message-circle" size={18} color="#FFFFFF" />
          <Text className="text-on-surface text-base font-semibold">Use in Chat</Text>
        </TouchableOpacity>

        {coach.is_system && (
          <TouchableOpacity
            className="flex-1 flex-row items-center justify-center py-3.5 rounded-xl gap-2"
            style={{
              ...glassCard,
              borderRadius: 12,
              borderColor: isHidden ? colors.pierre.violet : 'rgba(255, 255, 255, 0.1)',
            }}
            onPress={handleToggleHidden}
            disabled={isTogglingHidden}
            testID="hide-button"
          >
            {isTogglingHidden ? (
              <ActivityIndicator size="small" color={colors.text.secondary} />
            ) : (
              <>
                <Feather
                  name={isHidden ? 'eye' : 'eye-off'}
                  size={18}
                  color={isHidden ? colors.pierre.violet : colors.text.secondary}
                />
                <Text className="text-base font-medium" style={{ color: isHidden ? colors.pierre.violet : colors.text.secondary }}>
                  {isHidden ? 'Show' : 'Hide'}
                </Text>
              </>
            )}
          </TouchableOpacity>
        )}

        {!coach.is_system && (
          <TouchableOpacity
            className="flex-1 flex-row items-center justify-center py-3.5 rounded-xl gap-2"
            style={{
              ...glassCard,
              borderRadius: 12,
              borderColor: colors.error,
            }}
            onPress={handleDelete}
            disabled={isDeleting}
            testID="delete-button"
          >
            {isDeleting ? (
              <ActivityIndicator size="small" color={colors.error} />
            ) : (
              <>
                <Feather name="trash-2" size={18} color={colors.error} />
                <Text className="text-base font-medium" style={{ color: colors.error }}>Delete</Text>
              </>
            )}
          </TouchableOpacity>
        )}
      </View>
    </SafeAreaView>
  );
}
