// ABOUTME: Coach version history modal for viewing and reverting to previous versions
// ABOUTME: Displays timeline of versions with expand/collapse and revert functionality

import React, { useState, useCallback } from 'react';
import {
  View,
  Text,
  Modal,
  ScrollView,
  TouchableOpacity,
  ActivityIndicator,
  Alert,
  type ViewStyle,
} from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { useFocusEffect } from 'expo-router';
import { Ionicons } from '@expo/vector-icons';
import { PRIMARY_PALETTE, spacing, glassCard, useThemeColors } from '../../constants/theme';
import { coachesApi } from '../../services/api';
import type { FieldChange } from '../../types';

// Shadow style for version cards
const versionCardShadow: ViewStyle = {
  shadowColor: glassCard.shadowColor,
  shadowOffset: glassCard.shadowOffset,
  shadowOpacity: glassCard.shadowOpacity,
  shadowRadius: glassCard.shadowRadius,
  elevation: glassCard.elevation,
};

interface CoachVersionHistoryProps {
  coachId: string;
  coachTitle: string;
  isOpen: boolean;
  onClose: () => void;
  onReverted?: () => void;
}

interface VersionItem {
  version: number;
  content_snapshot: Record<string, unknown>;
  change_summary: string | null;
  created_at: string;
  created_by_name: string | null;
}

interface VersionsResponse {
  versions: VersionItem[];
  total: number;
  current_version: number;
}

/** What `GET /api/coaches/:id/versions/:from/diff/:to` answers with. */
interface VersionDiff {
  from_version: number;
  to_version: number;
  changes: FieldChange[];
}

/** Render one side of a field change, collapsing an absent value to a word. */
function diffValueText(value: unknown): string {
  if (value === null || value === undefined) return '(empty)';
  const text = typeof value === 'object' ? JSON.stringify(value, null, 2) : String(value);
  return text.length > 300 ? `${text.substring(0, 300)}…` : text;
}

export function CoachVersionHistory({
  coachId,
  coachTitle,
  isOpen,
  onClose,
  onReverted,
}: CoachVersionHistoryProps) {
  const colors = useThemeColors();
  const insets = useSafeAreaInsets();
  const [versionsData, setVersionsData] = useState<VersionsResponse | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isReverting, setIsReverting] = useState(false);
  const [selectedVersion, setSelectedVersion] = useState<VersionItem | null>(null);
  // Compare mode: the athlete picks two versions and the server reports what
  // changed between them, field by field. Without it a version's only story was
  // its own snapshot, which says nothing about what a given edit actually did.
  const [isComparing, setIsComparing] = useState(false);
  const [compareSelection, setCompareSelection] = useState<number[]>([]);
  const [diff, setDiff] = useState<VersionDiff | null>(null);
  const [isLoadingDiff, setIsLoadingDiff] = useState(false);

  const loadVersions = useCallback(async () => {
    try {
      setIsLoading(true);
      const response = await coachesApi.getVersions(coachId, 50);
      setVersionsData(response);
    } catch (error) {
      console.error('Failed to load versions:', error);
      Alert.alert('Error', 'Failed to load version history');
    } finally {
      setIsLoading(false);
    }
  }, [coachId]);

  // Load versions when modal opens
  useFocusEffect(
    useCallback(() => {
      if (isOpen) {
        loadVersions();
      }
    }, [isOpen, loadVersions])
  );

  const formatDate = (dateString: string): string => {
    const date = new Date(dateString);
    return date.toLocaleString(undefined, {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  };

  const loadDiff = useCallback(
    async (fromVersion: number, toVersion: number) => {
      try {
        setIsLoadingDiff(true);
        const response = await coachesApi.getVersionDiff(coachId, fromVersion, toVersion);
        setDiff(response);
      } catch (error) {
        console.error('Failed to load version diff:', error);
        Alert.alert('Error', 'Failed to compare versions');
      } finally {
        setIsLoadingDiff(false);
      }
    },
    [coachId],
  );

  const exitCompareMode = useCallback(() => {
    setIsComparing(false);
    setCompareSelection([]);
    setDiff(null);
  }, []);

  const enterCompareMode = useCallback(() => {
    setSelectedVersion(null);
    setDiff(null);
    setCompareSelection([]);
    setIsComparing(true);
  }, []);

  /**
   * Add or remove a version from the comparison.
   *
   * Two versions make a comparison, so picking a third replaces the older of
   * the pair rather than refusing the tap. The pair is always sent oldest to
   * newest so the reported change reads forwards in time.
   */
  const toggleCompareSelection = useCallback(
    (versionNumber: number) => {
      setDiff(null);
      setCompareSelection((current) => {
        if (current.includes(versionNumber)) {
          return current.filter((v) => v !== versionNumber);
        }
        const next = [...current, versionNumber].slice(-2);
        if (next.length === 2) {
          const [from, to] = [...next].sort((a, b) => a - b);
          void loadDiff(from, to);
        }
        return next;
      });
    },
    [loadDiff],
  );

  const handleVersionPress = (version: VersionItem) => {
    if (isComparing) {
      toggleCompareSelection(version.version);
      return;
    }
    if (selectedVersion?.version === version.version) {
      setSelectedVersion(null);
    } else {
      setSelectedVersion(version);
    }
  };

  const handleRevert = (version: VersionItem) => {
    Alert.alert(
      'Confirm Revert',
      `Are you sure you want to revert to version ${version.version}? This will create a new version with the reverted content. Your current changes will be preserved in the version history.`,
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Revert',
          style: 'destructive',
          onPress: () => confirmRevert(version.version),
        },
      ]
    );
  };

  const confirmRevert = async (versionNumber: number) => {
    try {
      setIsReverting(true);
      await coachesApi.revertToVersion(coachId, versionNumber);
      Alert.alert('Success', 'Coach reverted successfully');
      setSelectedVersion(null);
      onReverted?.();
      onClose();
    } catch (error) {
      console.error('Failed to revert:', error);
      Alert.alert('Error', 'Failed to revert to version');
    } finally {
      setIsReverting(false);
    }
  };

  const renderSnapshotField = (key: string, value: unknown) => {
    if (value === null || value === undefined) return null;

    const displayValue =
      typeof value === 'object' ? JSON.stringify(value, null, 2) : String(value);

    // Truncate very long values
    const truncatedValue =
      displayValue.length > 200 ? `${displayValue.substring(0, 200)}...` : displayValue;

    return (
      <View key={key} className="py-2 border-b border-border-subtle/30 last:border-0">
        <Text className="text-xs font-medium text-text-tertiary uppercase tracking-wide mb-1">
          {key.replace(/_/g, ' ')}
        </Text>
        <Text className="text-sm text-text-primary" numberOfLines={5}>
          {truncatedValue}
        </Text>
      </View>
    );
  };

  const renderVersionItem = (version: VersionItem) => {
    const isSelected = !isComparing && selectedVersion?.version === version.version;
    const isCurrent = versionsData?.current_version === version.version;
    const isPickedForCompare = compareSelection.includes(version.version);
    const isHighlighted = isSelected || isPickedForCompare;

    return (
      <View
        key={version.version}
        className={`rounded-lg mb-3 border ${
          isHighlighted ? 'border-primary-500 bg-primary-500/10' : 'border-border-subtle bg-background-secondary'
        }`}
        style={versionCardShadow}
      >
        {/* Version Header */}
        <TouchableOpacity
          className="flex-row items-center p-3"
          onPress={() => handleVersionPress(version)}
          testID={`version-item-${version.version}`}
        >
          <View className="w-10 h-10 rounded-full bg-background-tertiary items-center justify-center mr-3">
            <Text className="text-sm font-semibold text-text-primary">v{version.version}</Text>
          </View>
          <View className="flex-1">
            <View className="flex-row items-center gap-2">
              <Text className="text-base font-medium text-text-primary">
                {version.change_summary || 'Update'}
              </Text>
              {isCurrent && (
                <View className="px-2 py-0.5 rounded bg-primary-500/20">
                  <Text className="text-xs font-medium text-primary-400">Current</Text>
                </View>
              )}
            </View>
            <Text className="text-xs text-text-tertiary mt-0.5">
              {formatDate(version.created_at)}
              {version.created_by_name && ` by ${version.created_by_name}`}
            </Text>
          </View>
          <Ionicons
            name={
              isComparing
                ? isPickedForCompare
                  ? 'checkmark-circle'
                  : 'ellipse-outline'
                : isSelected
                  ? 'chevron-up'
                  : 'chevron-down'
            }
            size={20}
            color={isPickedForCompare ? PRIMARY_PALETTE[500] : colors.text.tertiary}
          />
        </TouchableOpacity>

        {/* Expanded Content */}
        {isSelected && (
          <View className="px-3 pb-3 border-t border-border-subtle/30">
            {/* Snapshot Content */}
            <View className="bg-background-primary/50 rounded-lg p-3 mt-3">
              <Text className="text-xs font-semibold text-text-secondary uppercase tracking-wide mb-2">
                Snapshot Content
              </Text>
              {Object.entries(version.content_snapshot).map(([key, value]) =>
                renderSnapshotField(key, value)
              )}
            </View>

            {/* Revert Button */}
            {!isCurrent && (
              <TouchableOpacity
                className={`mt-3 py-2 px-4 rounded-lg border border-warning self-end ${
                  isReverting ? 'opacity-50' : ''
                }`}
                onPress={() => handleRevert(version)}
                disabled={isReverting}
                testID={`revert-button-${version.version}`}
              >
                {isReverting ? (
                  <ActivityIndicator size="small" color={colors.warning} />
                ) : (
                  <Text className="text-sm font-medium text-warning">
                    Revert to v{version.version}
                  </Text>
                )}
              </TouchableOpacity>
            )}
          </View>
        )}
      </View>
    );
  };

  return (
    <Modal
      visible={isOpen}
      animationType="slide"
      transparent
      onRequestClose={onClose}
    >
      <View className="flex-1 bg-black/50 justify-end">
        <View
          className="bg-background-primary rounded-t-2xl max-h-[85%]"
          style={{ paddingBottom: insets.bottom + spacing.md }}
        >
          {/* Header */}
          <View className="flex-row items-center justify-between px-4 py-3 border-b border-border-subtle">
            <View className="flex-1">
              <Text className="text-lg font-semibold text-text-primary" numberOfLines={1}>
                Version History
              </Text>
              <Text className="text-sm text-text-secondary" numberOfLines={1}>
                {coachTitle}
              </Text>
            </View>
            <TouchableOpacity
              className="px-3 py-1.5 mr-1 rounded-lg border border-border-subtle"
              onPress={isComparing ? exitCompareMode : enterCompareMode}
              testID="toggle-compare-mode"
            >
              <Text className="text-sm font-medium text-text-primary">
                {isComparing ? 'Done' : 'Compare'}
              </Text>
            </TouchableOpacity>
            <TouchableOpacity
              className="p-2 -mr-2"
              onPress={onClose}
              testID="close-version-history"
            >
              <Ionicons name="close" size={24} color={colors.text.secondary} />
            </TouchableOpacity>
          </View>

          {/* Stats Bar */}
          {versionsData && (
            <View className="flex-row items-center justify-between px-4 py-2 bg-background-secondary/50">
              <Text className="text-sm text-text-secondary">
                {versionsData.total} version{versionsData.total !== 1 ? 's' : ''} saved
              </Text>
              <Text className="text-sm font-medium text-text-primary">
                Current: v{versionsData.current_version}
              </Text>
            </View>
          )}

          {/* Content */}
          {isLoading ? (
            <View className="flex-1 items-center justify-center py-12">
              <ActivityIndicator size="large" color={PRIMARY_PALETTE[500]} />
              <Text className="mt-3 text-text-secondary">Loading versions...</Text>
            </View>
          ) : !versionsData || versionsData.versions.length === 0 ? (
            <View className="flex-1 items-center justify-center py-12 px-6">
              <Ionicons name="git-branch-outline" size={48} color={colors.text.tertiary} />
              <Text className="text-base text-text-secondary text-center mt-3">
                No version history yet
              </Text>
              <Text className="text-sm text-text-tertiary text-center mt-1">
                Versions are created automatically when you update the coach.
              </Text>
            </View>
          ) : (
            <ScrollView
              className="flex-1 px-4 pt-3"
              showsVerticalScrollIndicator={false}
              testID="version-list"
            >
              {isComparing && (
                <View
                  className="rounded-lg mb-3 p-3 bg-background-secondary border border-border-subtle"
                  testID="version-compare-panel"
                >
                  {isLoadingDiff ? (
                    <ActivityIndicator size="small" color={PRIMARY_PALETTE[500]} />
                  ) : diff ? (
                    <>
                      <Text className="text-sm font-semibold text-text-primary mb-2">
                        v{diff.from_version} → v{diff.to_version}
                      </Text>
                      {diff.changes.length === 0 ? (
                        <Text className="text-sm text-text-secondary" testID="version-diff-empty">
                          No field changed between these versions.
                        </Text>
                      ) : (
                        diff.changes.map((change) => (
                          <View
                            key={change.field}
                            className="py-2 border-b border-border-subtle/30"
                            testID={`diff-field-${change.field}`}
                          >
                            <Text className="text-xs font-medium text-text-tertiary uppercase tracking-wide mb-1">
                              {change.field.replace(/_/g, ' ')}
                            </Text>
                            <Text className="text-sm text-error" numberOfLines={4}>
                              − {diffValueText(change.old_value)}
                            </Text>
                            <Text className="text-sm text-success mt-1" numberOfLines={4}>
                              + {diffValueText(change.new_value)}
                            </Text>
                          </View>
                        ))
                      )}
                    </>
                  ) : (
                    <Text className="text-sm text-text-secondary" testID="version-compare-hint">
                      Pick two versions to compare.
                    </Text>
                  )}
                </View>
              )}
              {versionsData.versions.map(renderVersionItem)}
              <View className="h-4" />
            </ScrollView>
          )}
        </View>
      </View>
    </Modal>
  );
}

export default CoachVersionHistory;
