// ABOUTME: Coach library screen for managing the athlete's installed coaches, reached from Discover
// ABOUTME: Lists coaches with their @handle, category filters, favorites toggle, import/export and CRUD actions

import React, { useState, useCallback, useMemo } from 'react';
import {
  View,
  Text,
  ScrollView,
  TouchableOpacity,
  ActivityIndicator,
  Alert,
  Modal,
  RefreshControl,
  KeyboardAvoidingView,
  Platform,
  type ViewStyle,
} from 'react-native';
import { FlashList } from '@shopify/flash-list';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useFocusEffect, useRouter } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import { LinearGradient } from 'expo-linear-gradient';
import * as DocumentPicker from 'expo-document-picker';
import * as Sharing from 'expo-sharing';
import { File, Paths } from 'expo-file-system';
import { PRIMARY_PALETTE, spacing, glassCard, gradients, useThemeColors } from '../../constants/theme';
import { coachesApi } from '../../services/api';
import { COACH_DETAIL_ROUTE, COACH_EDITOR_ROUTE } from '../../navigation/routes';
import { useAuth } from '../../contexts/AuthContext';
import { FloatingSearchBar, PromptDialog, ScrollFadeContainer, SwipeableRow, type SwipeAction } from '../../components/ui';
import type {
  Coach,
  CoachCategory,
  ImportCoachResponse,
  ImportPreviewResponse,
} from '../../types';

/** What the confirm sheet will send once the athlete accepts the preview. */
type PendingImport =
  | { kind: 'markdown'; markdown: string; source: string }
  | { kind: 'url'; url: string; source: string };

/** File name an exported coach is shared under. */
function exportFileName(title: string): string {
  const slug = title
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return `${slug || 'coach'}.md`;
}

// Category filter options
const CATEGORY_FILTERS: Array<{ key: CoachCategory | 'all'; label: string }> = [
  { key: 'all', label: 'All' },
  { key: 'training', label: 'Training' },
  { key: 'nutrition', label: 'Nutrition' },
  { key: 'recovery', label: 'Recovery' },
  { key: 'recipes', label: 'Recipes' },
  { key: 'mobility', label: 'Mobility' },
  { key: 'custom', label: 'Custom' },
];

// Source filter options (user-created vs system coaches)
type CoachSource = 'all' | 'user' | 'system';
const SOURCE_FILTERS: Array<{ key: CoachSource; label: string }> = [
  { key: 'all', label: 'All Sources' },
  { key: 'user', label: 'My Coaches' },
  { key: 'system', label: 'System' },
];

// Coach category colors matching Stitch UX spec
const COACH_CATEGORY_COLORS: Record<string, string> = {
  training: '#3c6658',  // Green per Stitch spec
  nutrition: '#8f6a2e', // Amber per Stitch spec
  recovery: '#0d3b2e',  // Cyan per Stitch spec
  recipes: '#8f6a2e',   // Amber
  mobility: '#7a4d5e',  // Pink - for stretching/yoga
  custom: '#00241a',    // Violet per Stitch spec
};

export function CoachLibraryScreen() {
  const colors = useThemeColors();
  // Action menu with glassmorphism style
  const actionMenuStyle: ViewStyle = useMemo(() => ({
    backgroundColor: colors.background.elevated,
    borderRadius: 16,
    paddingVertical: spacing.sm,
    minWidth: 240,
    borderWidth: 1,
    borderColor: colors.border.subtle,
    shadowColor: colors.text.primary,
    shadowOffset: { width: 0, height: 8 },
    shadowOpacity: 0.18,
    shadowRadius: 24,
    elevation: 12,
    overflow: 'hidden',
  }), [colors]);
  const router = useRouter();
  const { isAuthenticated } = useAuth();
  const [coaches, setCoaches] = useState<Coach[]>([]);
  const [selectedCategory, setSelectedCategory] = useState<CoachCategory | 'all'>('all');
  const [selectedSource, setSelectedSource] = useState<CoachSource>('all');
  const [showFavoritesOnly, setShowFavoritesOnly] = useState(false);
  const [showHidden, setShowHidden] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [actionMenuVisible, setActionMenuVisible] = useState(false);
  const [selectedCoach, setSelectedCoach] = useState<Coach | null>(null);
  const [renamePromptVisible, setRenamePromptVisible] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [importUrlPromptVisible, setImportUrlPromptVisible] = useState(false);
  const [importPreview, setImportPreview] = useState<ImportPreviewResponse | null>(null);
  const [pendingImport, setPendingImport] = useState<PendingImport | null>(null);
  const [isImporting, setIsImporting] = useState(false);

  const loadCoaches = useCallback(async (isRefresh = false) => {
    if (!isAuthenticated) return;

    try {
      if (isRefresh) {
        setIsRefreshing(true);
      } else {
        setIsLoading(true);
      }
      setLoadError(null);

      // Always load all coaches (including hidden) and hidden list in parallel
      // We filter locally based on showHidden state to preserve local changes
      const [coachesResponse, hiddenResponse] = await Promise.all([
        coachesApi.list({ include_hidden: true }),
        coachesApi.getHidden(),
      ]);

      // Create a set of hidden coach IDs for quick lookup
      const hiddenIds = new Set((hiddenResponse.coaches || []).map((c: { id: string }) => c.id));

      // Mark coaches as hidden if they're in the hidden list
      const coachesWithHiddenFlag = coachesResponse.coaches.map((coach: Coach) => ({
        ...coach,
        is_hidden: hiddenIds.has(coach.id),
      }));

      // Sort: favorites first, then by use_count descending
      const sorted = [...coachesWithHiddenFlag].sort((a, b) => {
        if (a.is_favorite !== b.is_favorite) {
          return a.is_favorite ? -1 : 1;
        }
        return b.use_count - a.use_count;
      });
      setCoaches(sorted);
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : 'Failed to load coaches';
      setLoadError(errorMessage);
      console.error('Failed to load coaches:', err);
    } finally {
      setIsLoading(false);
      setIsRefreshing(false);
    }
  }, [isAuthenticated]);

  // Compute filtered coaches - derived state using useMemo to avoid act() warnings in tests
  const filteredCoaches = useMemo(() => {
    let filtered = [...coaches];

    // Filter out hidden coaches unless showHidden is enabled
    if (!showHidden) {
      filtered = filtered.filter((coach) => !coach.is_hidden);
    }

    // Filter by category
    if (selectedCategory !== 'all') {
      filtered = filtered.filter((coach) => coach.category === selectedCategory);
    }

    // Filter by source (user-created vs system)
    if (selectedSource === 'user') {
      filtered = filtered.filter((coach) => !coach.is_system);
    } else if (selectedSource === 'system') {
      filtered = filtered.filter((coach) => coach.is_system);
    }

    // Filter favorites only
    if (showFavoritesOnly) {
      filtered = filtered.filter((coach) => coach.is_favorite);
    }

    // Filter by search query
    if (searchQuery.trim()) {
      const query = searchQuery.toLowerCase();
      filtered = filtered.filter((coach) =>
        coach.title.toLowerCase().includes(query) ||
        (coach.description || '').toLowerCase().includes(query)
      );
    }

    return filtered;
  }, [coaches, selectedCategory, selectedSource, showFavoritesOnly, showHidden, searchQuery]);

  useFocusEffect(
    useCallback(() => {
      loadCoaches();
    }, [loadCoaches])
  );

  const handleRefresh = () => {
    loadCoaches(true);
  };

  const handleCoachPress = (coach: Coach) => {
    router.push({ pathname: COACH_DETAIL_ROUTE, params: { coachId: coach.id } });
  };

  const handleCoachLongPress = (coach: Coach) => {
    setSelectedCoach(coach);
    setActionMenuVisible(true);
  };

  const handleCreateCoach = () => {
    router.push({ pathname: COACH_EDITOR_ROUTE });
  };

  const handleToggleFavorite = async (coach?: Coach) => {
    const targetCoach = coach ?? selectedCoach;
    if (!targetCoach) return;
    setActionMenuVisible(false);

    try {
      const result = await coachesApi.toggleFavorite(targetCoach.id);
      setCoaches((prev) =>
        prev.map((c) =>
          c.id === targetCoach.id ? { ...c, is_favorite: result.is_favorite } : c
        )
      );
    } catch (error) {
      console.error('Failed to toggle favorite:', error);
      Alert.alert('Error', 'Failed to update favorite status');
    }
  };

  const handleRename = () => {
    if (!selectedCoach) return;
    setActionMenuVisible(false);
    setRenamePromptVisible(true);
  };

  const handleRenameSubmit = async (newTitle: string) => {
    setRenamePromptVisible(false);
    if (!selectedCoach) return;

    try {
      const updated = await coachesApi.update(selectedCoach.id, {
        title: newTitle,
      });
      setCoaches((prev) =>
        prev.map((c) => (c.id === selectedCoach.id ? { ...c, title: updated.title } : c))
      );
    } catch (error) {
      console.error('Failed to rename coach:', error);
      Alert.alert('Error', 'Failed to rename coach');
    } finally {
      setSelectedCoach(null);
    }
  };

  const handleRenameCancel = () => {
    setRenamePromptVisible(false);
    setSelectedCoach(null);
  };

  const handleDelete = () => {
    if (!selectedCoach) return;
    setActionMenuVisible(false);

    Alert.alert(
      'Delete Coach',
      `Are you sure you want to delete "${selectedCoach.title}"? This cannot be undone.`,
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Delete',
          style: 'destructive',
          onPress: async () => {
            try {
              await coachesApi.delete(selectedCoach.id);
              setCoaches((prev) => prev.filter((c) => c.id !== selectedCoach.id));
            } catch (error) {
              console.error('Failed to delete coach:', error);
              Alert.alert('Error', 'Failed to delete coach');
            }
          },
        },
      ]
    );
  };

  const handleHideCoach = async (coach?: Coach) => {
    const targetCoach = coach ?? selectedCoach;
    if (!targetCoach) return;
    setActionMenuVisible(false);

    try {
      await coachesApi.hide(targetCoach.id);
      // Remove from list if not showing hidden coaches, otherwise update the flag
      if (showHidden) {
        setCoaches((prev) =>
          prev.map((c) => (c.id === targetCoach.id ? { ...c, is_hidden: true } : c))
        );
      } else {
        setCoaches((prev) => prev.filter((c) => c.id !== targetCoach.id));
      }
    } catch (error) {
      console.error('Failed to hide coach:', error);
      Alert.alert('Error', 'Failed to hide coach');
    }
  };

  const handleShowCoach = async (coach?: Coach) => {
    const targetCoach = coach ?? selectedCoach;
    if (!targetCoach) return;
    setActionMenuVisible(false);

    try {
      await coachesApi.show(targetCoach.id);
      // Update main coaches list - add if not present, update if present
      setCoaches((prev) => {
        const exists = prev.some((c) => c.id === targetCoach.id);
        if (exists) {
          return prev.map((c) => (c.id === targetCoach.id ? { ...c, is_hidden: false } : c));
        }
        // Coach was only loaded via include_hidden, add it to main list
        return [...prev, { ...targetCoach, is_hidden: false }];
      });
    } catch (error) {
      console.error('Failed to show coach:', error);
      Alert.alert('Error', 'Failed to show coach');
    }
  };

  const handleForkCoach = (coach?: Coach) => {
    const targetCoach = coach ?? selectedCoach;
    if (!targetCoach || !targetCoach.is_system) return;
    setActionMenuVisible(false);

    Alert.alert(
      'Fork Coach',
      `Create your own copy of "${targetCoach.title}"? You can customize the forked coach however you like.`,
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Fork',
          onPress: async () => {
            try {
              const result = await coachesApi.fork(targetCoach.id);
              // Add the new forked coach to the list
              setCoaches((prev) => [result.coach, ...prev]);
              // Navigate to wizard to customize
              router.push({ pathname: COACH_EDITOR_ROUTE, params: { coachId: result.coach.id } });
            } catch (error) {
              console.error('Failed to fork coach:', error);
              Alert.alert('Error', 'Failed to fork coach. Please try again.');
            }
          },
        },
      ]
    );
  };

  /**
   * Preview a markdown coach before it is written.
   *
   * The server parses the document and reports what it found, whether it is
   * valid, and whether an identical coach already exists — all before anything
   * is saved. Showing that first is why an import can be confirmed rather than
   * discovered afterwards.
   */
  const previewImport = useCallback(async (pending: PendingImport) => {
    try {
      setIsImporting(true);
      const preview =
        pending.kind === 'markdown'
          ? await coachesApi.importPreview(pending.markdown)
          : ((await coachesApi.importFromUrl(pending.url, false)) as ImportPreviewResponse);
      setPendingImport(pending);
      setImportPreview(preview);
    } catch (error) {
      console.error('Failed to preview coach import:', error);
      Alert.alert('Import Failed', 'Could not read that coach document.');
    } finally {
      setIsImporting(false);
    }
  }, []);

  const handleImportFromFile = useCallback(async () => {
    const result = await DocumentPicker.getDocumentAsync({
      type: ['text/markdown', 'text/plain', 'application/octet-stream'],
      copyToCacheDirectory: true,
    });
    if (result.canceled) return;
    const asset = result.assets[0];
    if (!asset) return;

    try {
      const markdown = await new File(asset.uri).text();
      await previewImport({ kind: 'markdown', markdown, source: asset.name });
    } catch (error) {
      console.error('Failed to read coach file:', error);
      Alert.alert('Import Failed', 'Could not read that file.');
    }
  }, [previewImport]);

  const handleImportFromUrlSubmit = useCallback(
    async (url: string) => {
      setImportUrlPromptVisible(false);
      const trimmed = url.trim();
      if (!trimmed) return;
      await previewImport({ kind: 'url', url: trimmed, source: trimmed });
    },
    [previewImport],
  );

  const handleImportPress = useCallback(() => {
    Alert.alert('Import Coach', 'Where is the coach document?', [
      { text: 'From a file', onPress: () => void handleImportFromFile() },
      { text: 'From a URL', onPress: () => setImportUrlPromptVisible(true) },
      { text: 'Cancel', style: 'cancel' },
    ]);
  }, [handleImportFromFile]);

  const handleConfirmImport = useCallback(async () => {
    if (!pendingImport) return;
    try {
      setIsImporting(true);
      const result =
        pendingImport.kind === 'markdown'
          ? await coachesApi.importFromMarkdown(pendingImport.markdown)
          : ((await coachesApi.importFromUrl(pendingImport.url, true)) as ImportCoachResponse);
      setCoaches((prev) => [result.coach, ...prev]);
      setImportPreview(null);
      setPendingImport(null);
      Alert.alert('Coach Imported', `"${result.coach.title}" is now in your library.`);
    } catch (error) {
      console.error('Failed to import coach:', error);
      Alert.alert('Import Failed', 'The coach could not be imported.');
    } finally {
      setIsImporting(false);
    }
  }, [pendingImport]);

  const cancelImport = useCallback(() => {
    setImportPreview(null);
    setPendingImport(null);
  }, []);

  /**
   * Export one coach as the markdown document the importer accepts.
   *
   * Written into the cache directory and handed to the system share sheet,
   * which is the only way a file leaves the app on iOS.
   */
  const handleExportCoach = useCallback(async (coach?: Coach) => {
    const targetCoach = coach ?? selectedCoach;
    if (!targetCoach) return;
    setActionMenuVisible(false);

    try {
      const markdown = await coachesApi.exportAsMarkdown(targetCoach.id);
      const file = new File(Paths.cache, exportFileName(targetCoach.title));
      file.create({ overwrite: true });
      file.write(markdown);
      await Sharing.shareAsync(file.uri, {
        mimeType: 'text/markdown',
        UTI: 'net.daringfireball.markdown',
        dialogTitle: `Export ${targetCoach.title}`,
      });
    } catch (error) {
      console.error('Failed to export coach:', error);
      Alert.alert('Export Failed', 'The coach could not be exported.');
    }
  }, [selectedCoach]);

  const closeActionMenu = () => {
    setActionMenuVisible(false);
    setSelectedCoach(null);
  };

  const renderCoachCard = ({ item }: { item: Coach }) => {
    const isHidden = item.is_hidden;
    const categoryColor = COACH_CATEGORY_COLORS[item.category];

    const leftActions: SwipeAction[] = [
      {
        icon: 'heart',
        label: item.is_favorite ? 'Unfave' : 'Favorite',
        color: '#FFFFFF',
        backgroundColor: '#8f6a2e',
        onPress: () => handleToggleFavorite(item),
      },
    ];

    const rightActions: SwipeAction[] = [
      {
        icon: 'message-circle',
        label: 'Chat',
        color: colors.tokens.onPrimary,
        backgroundColor: colors.pierre.violet,
        onPress: () => handleCoachPress(item),
      },
    ];

    return (
      <SwipeableRow
        leftActions={leftActions}
        rightActions={rightActions}
        testID={`swipeable-coach-${item.id}`}
      >
      <TouchableOpacity
        style={[
          {
            ...glassCard,
            backgroundColor: colors.background.elevated,
            borderColor: colors.border.subtle,
            shadowColor: colors.text.primary,
            borderRadius: 16,
            overflow: 'hidden',
          },
          isHidden && { opacity: 0.6 },
        ]}
        onPress={() => handleCoachPress(item)}
        onLongPress={() => handleCoachLongPress(item)}
        delayLongPress={300}
        activeOpacity={0.7}
        accessible={true}
        accessibilityLabel={item.title}
        testID={`coach-card-${item.id}`}
      >
        {/* Category-colored gradient accent bar */}
        <LinearGradient
          colors={[categoryColor, `${categoryColor}80`] as [string, string]}
          start={{ x: 0, y: 0 }}
          end={{ x: 1, y: 0 }}
          style={{ height: 3, width: '100%' }}
        />
        <View className="flex-row items-start p-4">
          {/* Coach Avatar/Icon */}
          <View
            className="w-12 h-12 rounded-xl items-center justify-center mr-3"
            style={{ backgroundColor: `${categoryColor}20` }}
          >
            <Text className="text-xl">
              {item.category === 'training' ? '🏃' :
               item.category === 'nutrition' ? '🥗' :
               item.category === 'recovery' ? '😴' :
               item.category === 'recipes' ? '👨‍🍳' :
               item.category === 'mobility' ? '🧘' : '⚙️'}
            </Text>
          </View>

          <View className="flex-1">
            {/* Title and badges row */}
            <View className="flex-row items-center gap-2 mb-1">
              <Text className={`text-base font-semibold ${isHidden ? 'text-outline' : 'text-on-surface'}`} numberOfLines={1}>
                {item.title}
              </Text>
              {/* Category badge with color per Stitch spec */}
              <View
                className="px-2 py-0.5 rounded-full"
                style={{ backgroundColor: `${categoryColor}20` }}
              >
                <Text className="text-xs font-medium" style={{ color: categoryColor }}>
                  {item.category.charAt(0).toUpperCase() + item.category.slice(1)}
                </Text>
              </View>
            </View>

            {/* The @handle a mention or /coach invite addresses this coach by */}
            {item.handle && (
              <Text
                className="text-xs mb-1"
                style={{ color: colors.pierre.violet }}
                numberOfLines={1}
                testID={`coach-handle-${item.id}`}
              >
                @{item.handle}
              </Text>
            )}

            {/* Star rating (use count as proxy) and favorite button */}
            <View className="flex-row items-center gap-1 mb-1">
              {[1, 2, 3, 4, 5].map((star) => (
                <Feather
                  key={star}
                  name="star"
                  size={12}
                  color={item.use_count >= star * 2 ? '#8f6a2e' : colors.text.tertiary}
                />
              ))}
              <TouchableOpacity
                className="ml-2 p-0.5"
                onPress={() => handleToggleFavorite(item)}
                hitSlop={{ top: 10, bottom: 10, left: 10, right: 10 }}
                testID={`favorite-button-${item.id}`}
              >
                <Feather
                  name="heart"
                  size={14}
                  color={item.is_favorite ? colors.pierre.violet : colors.text.tertiary}
                />
              </TouchableOpacity>
            </View>

            {/* Description */}
            {item.description && (
              <Text className={`text-sm leading-5 ${isHidden ? 'text-on-surface-variant' : 'text-on-surface-variant'}`} numberOfLines={2}>
                {item.description}
              </Text>
            )}
          </View>

          {/* Chat CTA — primary surface, accessible label color, soft halo */}
          <TouchableOpacity
            className="px-4 py-2 rounded-full ml-2"
            style={{
              backgroundColor: colors.pierre.violet,
              shadowColor: colors.pierre.violet,
              shadowOffset: { width: 0, height: 2 },
              shadowOpacity: 0.25,
              shadowRadius: 8,
              elevation: 4,
            }}
            onPress={() => handleCoachPress(item)}
            testID={`chat-button-${item.id}`}
          >
            <Text className="text-sm font-semibold" style={{ color: colors.tokens.onPrimary }}>
              Chat
            </Text>
          </TouchableOpacity>
        </View>

        {/* Action row for system coaches and hidden coaches (inside card content) */}
        {(item.is_system || isHidden) && (
          <View className="flex-row items-center justify-end mx-4 mb-3 pt-2 border-t ghost-border gap-2">
            {/* Fork button for system coaches */}
            {item.is_system && (
              <TouchableOpacity
                className="flex-row items-center px-2 py-1 rounded"
                style={{ backgroundColor: colors.background.tertiary }}
                onPress={() => handleForkCoach(item)}
                hitSlop={{ top: 10, bottom: 10, left: 10, right: 10 }}
                testID={`fork-button-${item.id}`}
              >
                <Feather name="copy" size={14} color={colors.text.tertiary} />
                <Text className="text-xs text-outline ml-1">Fork</Text>
              </TouchableOpacity>
            )}
            {/* Hide/Show button */}
            {item.is_system && (
              <TouchableOpacity
                className="flex-row items-center px-2 py-1 rounded"
                style={{ backgroundColor: colors.background.tertiary }}
                onPress={() => {
                  if (isHidden) {
                    handleShowCoach(item);
                  } else {
                    handleHideCoach(item);
                  }
                }}
                hitSlop={{ top: 10, bottom: 10, left: 10, right: 10 }}
                testID={`hide-button-${item.id}`}
              >
                <Feather
                  name={isHidden ? 'eye' : 'eye-off'}
                  size={14}
                  color={isHidden ? colors.pierre.violet : colors.text.tertiary}
                />
                <Text className="text-xs text-outline ml-1">{isHidden ? 'Show' : 'Hide'}</Text>
              </TouchableOpacity>
            )}
            {/* Hidden indicator */}
            {isHidden && !item.is_system && (
              <View className="flex-row items-center">
                <Feather name="eye-off" size={14} color={colors.text.tertiary} />
                <Text className="text-xs text-outline ml-1">Hidden</Text>
              </View>
            )}
          </View>
        )}
      </TouchableOpacity>
      </SwipeableRow>
    );
  };

  const renderCategoryFilter = () => (
    <View className="flex-row items-center py-3 border-b border-border-subtle">
      <ScrollFadeContainer
        backgroundColor={colors.background.primary}
        fadeWidth={40}
        contentContainerStyle={{ paddingHorizontal: spacing.md, gap: spacing.sm }}
        testID="category-filter-scroll"
      >
        {CATEGORY_FILTERS.map((filter) => (
          <TouchableOpacity
            key={filter.key}
            className="px-4 py-2 rounded-full"
            style={{
              backgroundColor: selectedCategory === filter.key
                ? colors.pierre.violet
                : colors.background.tertiary,
              borderWidth: 1,
              borderColor: selectedCategory === filter.key
                ? colors.pierre.violet
                : colors.border.default,
            }}
            onPress={() => setSelectedCategory(filter.key)}
            testID={`category-filter-${filter.key}`}
          >
            <Text
              className={`text-sm ${
                selectedCategory === filter.key
                  ? 'font-semibold'
                  : 'text-on-surface-variant'
              }`}
              style={
                selectedCategory === filter.key
                  ? { color: colors.tokens.onPrimary }
                  : undefined
              }
            >
              {filter.label}
            </Text>
          </TouchableOpacity>
        ))}
      </ScrollFadeContainer>
    </View>
  );

  const renderSourceFilter = () => (
    <View className="flex-row justify-center items-center py-2 px-4 gap-3">
      {SOURCE_FILTERS.map((filter) => (
        <TouchableOpacity
          key={filter.key}
          className="px-3 py-1.5 rounded-full"
          style={{
            backgroundColor: selectedSource === filter.key
              ? colors.background.tertiary
              : 'transparent',
            borderWidth: 1,
            borderColor: selectedSource === filter.key
              ? colors.pierre.violet
              : 'transparent',
          }}
          onPress={() => setSelectedSource(filter.key)}
          testID={`source-filter-${filter.key}`}
        >
          <Text
            className={`text-xs ${
              selectedSource === filter.key
                ? 'font-semibold'
                : ''
            }`}
            style={{
              color: selectedSource === filter.key
                ? colors.pierre.violet
                : colors.text.secondary,
            }}
          >
            {filter.label}
          </Text>
        </TouchableOpacity>
      ))}
    </View>
  );

  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="coach-library-screen">
    <KeyboardAvoidingView
      className="flex-1"
      behavior={Platform.OS === 'ios' ? 'padding' : undefined}
    >
      {/* Header with back to Discover, bold title and action buttons */}
      <View className="flex-row items-center px-4 py-3 border-b border-border-subtle">
        <TouchableOpacity
          className="w-10 h-10 items-center justify-center -ml-2 mr-1"
          onPress={() => router.back()}
          accessibilityRole="button"
          accessibilityLabel="Back to Discover"
          testID="back-button"
        >
          <Feather name="arrow-left" size={24} color={colors.text.primary} />
        </TouchableOpacity>
        <Text className="flex-1 text-xl font-bold text-on-surface">My coaches</Text>
        <View className="flex-row items-center gap-2">
          <TouchableOpacity
            className={`w-10 h-10 items-center justify-center rounded-full ${showFavoritesOnly ? 'bg-pierre-violet/20' : ''}`}
            onPress={() => setShowFavoritesOnly(!showFavoritesOnly)}
            hitSlop={{ top: 10, bottom: 10, left: 10, right: 10 }}
            testID="favorites-toggle"
          >
            <Feather
              name="star"
              size={20}
              color={showFavoritesOnly ? '#8f6a2e' : colors.text.tertiary}
            />
          </TouchableOpacity>
          <TouchableOpacity
            className={`w-10 h-10 items-center justify-center rounded-full ${showHidden ? 'bg-pierre-violet/20' : ''}`}
            onPress={() => setShowHidden(!showHidden)}
            hitSlop={{ top: 10, bottom: 10, left: 10, right: 10 }}
            testID="show-hidden-toggle"
          >
            <Feather
              name={showHidden ? 'eye' : 'eye-off'}
              size={20}
              color={showHidden ? colors.pierre.violet : colors.text.tertiary}
            />
          </TouchableOpacity>
          {/* Create coach button - matches Chat tab style */}
          <TouchableOpacity
            className="w-10 h-10 items-center justify-center bg-background-tertiary rounded-lg"
            onPress={handleCreateCoach}
            testID="create-coach-button"
          >
            <Text className="text-2xl text-text-primary font-light">+</Text>
          </TouchableOpacity>
        </View>
      </View>

      {/* Category Filter */}
      {renderCategoryFilter()}

      {/* Source Filter (User vs System) */}
      {renderSourceFilter()}

      {/* Import / export action row. Import lives here because it creates a
          coach rather than acting on one; export is per-coach and sits in that
          coach's action menu, next to the other single-coach actions. */}
      <View
        className="flex-row items-center px-4 py-2 gap-3"
        testID="coach-import-export-row"
      >
        <TouchableOpacity
          className="flex-row items-center gap-2 px-3 py-2 rounded-lg border border-border-subtle"
          onPress={handleImportPress}
          disabled={isImporting}
          testID="import-coach-button"
        >
          {isImporting && !importPreview ? (
            <ActivityIndicator size="small" color={colors.text.secondary} />
          ) : (
            <Feather name="upload" size={16} color={colors.text.secondary} />
          )}
          <Text className="text-sm font-semibold text-text-secondary">Import coach</Text>
        </TouchableOpacity>
        <Text className="flex-1 text-xs text-text-tertiary">
          Markdown file or URL. Export any coach from its ··· menu.
        </Text>
      </View>

      {/* Load Error Display */}
      {loadError && (
        <View className="mx-4 mt-2 p-3 bg-error/10 border border-error/30 rounded-lg flex-row items-center justify-between">
          <Text className="flex-1 text-error text-sm mr-3">{loadError}</Text>
          <TouchableOpacity
            className="px-3 py-1.5 bg-error/20 rounded-md"
            onPress={() => {
              setLoadError(null);
              loadCoaches();
            }}
          >
            <Text className="text-error text-sm font-semibold">Retry</Text>
          </TouchableOpacity>
        </View>
      )}

      {/* Coaches List — FlashList stays mounted to avoid re-mount progressive rendering delays.
         FlashList v2 requires a valid parent size at mount time (Issue #483) and goes through
         a measurement cycle on fresh mount, which can cause items to not appear in the
         accessibility tree for UI testing tools. Overlaying ActivityIndicator keeps FlashList
         mounted so it updates data in-place without re-triggering the measurement cycle. */}
      <View className="flex-1">
        {isLoading && (
          <View className="absolute inset-0 z-10 items-center justify-center bg-background-primary">
            <ActivityIndicator size="large" color={PRIMARY_PALETTE[500]} />
          </View>
        )}
        <FlashList
          data={filteredCoaches}
          renderItem={renderCoachCard}
          keyExtractor={(item) => item.id}
          drawDistance={500}
          contentContainerStyle={{ padding: spacing.md, paddingBottom: 100 }}
          showsVerticalScrollIndicator={false}
          refreshControl={
            <RefreshControl
              refreshing={isRefreshing}
              onRefresh={handleRefresh}
              tintColor={PRIMARY_PALETTE[500]}
            />
          }
          ListEmptyComponent={
            !isLoading ? (
              <View className="items-center justify-center pt-12 px-5">
                <Text className="text-lg font-semibold text-text-primary mb-2 text-center">
                  {showFavoritesOnly
                    ? 'No favorite coaches'
                    : selectedSource === 'user'
                    ? 'No user-created coaches'
                    : selectedSource === 'system'
                    ? 'No system coaches'
                    : selectedCategory !== 'all'
                    ? `No ${selectedCategory} coaches`
                    : 'No coaches yet'}
                </Text>
                <Text className="text-base text-text-tertiary text-center">
                  {coaches.length === 0
                    ? 'Create your first coach to customize how Dravr helps you.'
                    : 'Try adjusting your filters.'}
                </Text>
              </View>
            ) : null
          }
        />
      </View>

    </KeyboardAvoidingView>

      {/* Floating search bar — transparent background, only pill visible */}
      <FloatingSearchBar
        value={searchQuery}
        onChangeText={setSearchQuery}
        placeholder="Search coaches..."
        testID="coach-search-input"
      />

      {/* Action Menu Modal */}
      <Modal
        visible={actionMenuVisible}
        animationType="fade"
        transparent
        onRequestClose={closeActionMenu}
      >
        <TouchableOpacity
          className="flex-1 bg-black/30 justify-center items-center"
          activeOpacity={1}
          onPress={closeActionMenu}
        >
          <View style={actionMenuStyle}>
            {/* Gradient accent bar */}
            <LinearGradient
              colors={gradients.violetCyan as [string, string]}
              start={{ x: 0, y: 0 }}
              end={{ x: 1, y: 0 }}
              style={{ height: 3, width: '100%', marginBottom: spacing.xs }}
            />
            <TouchableOpacity className="flex-row items-center px-4 py-2.5" onPress={() => handleToggleFavorite()}>
              <View className="w-6 mr-2 items-center">
                <Feather
                  name="star"
                  size={18}
                  color={selectedCoach?.is_favorite ? '#8f6a2e' : colors.text.primary}
                />
              </View>
              <Text className="text-base text-text-primary">
                {selectedCoach?.is_favorite ? 'Remove from favorites' : 'Add to favorites'}
              </Text>
            </TouchableOpacity>

            {/* Hide/Show option for system or assigned coaches */}
            {(selectedCoach?.is_system || selectedCoach?.is_assigned) && (
              <TouchableOpacity
                className="flex-row items-center px-4 py-2.5"
                onPress={() => (selectedCoach?.is_hidden ? handleShowCoach() : handleHideCoach())}
              >
                <View className="w-6 mr-2 items-center">
                  <Feather
                    name={selectedCoach?.is_hidden ? 'eye' : 'eye-off'}
                    size={18}
                    color={colors.text.primary}
                  />
                </View>
                <Text className="text-base text-text-primary">
                  {selectedCoach?.is_hidden ? 'Show coach' : 'Hide coach'}
                </Text>
              </TouchableOpacity>
            )}

            {/* Fork option for system coaches */}
            {selectedCoach?.is_system && (
              <TouchableOpacity
                className="flex-row items-center px-4 py-2.5"
                onPress={() => handleForkCoach()}
              >
                <View className="w-6 mr-2 items-center">
                  <Feather name="copy" size={18} color={colors.text.primary} />
                </View>
                <Text className="text-base text-text-primary">Fork (create my copy)</Text>
              </TouchableOpacity>
            )}

            {/* Export works for every coach, forked or not — the markdown is
                what the importer on any surface reads back. */}
            <TouchableOpacity
              className="flex-row items-center px-4 py-2.5"
              onPress={() => void handleExportCoach()}
              testID="export-coach-button"
            >
              <View className="w-6 mr-2 items-center">
                <Feather name="download" size={18} color={colors.text.primary} />
              </View>
              <Text className="text-base text-text-primary">Export as Markdown</Text>
            </TouchableOpacity>

            {/* Rename only for user-created coaches */}
            {!selectedCoach?.is_system && (
              <TouchableOpacity className="flex-row items-center px-4 py-2.5" onPress={handleRename}>
                <View className="w-6 mr-2 items-center">
                  <Feather name="edit-2" size={18} color={colors.text.primary} />
                </View>
                <Text className="text-base text-text-primary">Rename</Text>
              </TouchableOpacity>
            )}

            {/* Delete only for user-created coaches */}
            {!selectedCoach?.is_system && (
              <TouchableOpacity className="flex-row items-center px-4 py-2.5" onPress={handleDelete}>
                <View className="w-6 mr-2 items-center">
                  <Feather name="trash-2" size={18} color={colors.error} />
                </View>
                <Text className="text-base text-error">Delete</Text>
              </TouchableOpacity>
            )}
          </View>
        </TouchableOpacity>
      </Modal>

      {/* Rename Coach Prompt Dialog */}
      <PromptDialog
        visible={renamePromptVisible}
        title="Rename Coach"
        message="Enter a new name for this coach"
        defaultValue={selectedCoach?.title || ''}
        submitText="Save"
        cancelText="Cancel"
        onSubmit={handleRenameSubmit}
        onCancel={handleRenameCancel}
        testID="rename-coach-dialog"
      />

      {/* Import-from-URL prompt */}
      <PromptDialog
        visible={importUrlPromptVisible}
        title="Import from URL"
        message="Paste a link to a coach markdown document"
        defaultValue=""
        submitText="Preview"
        cancelText="Cancel"
        onSubmit={(url) => void handleImportFromUrlSubmit(url)}
        onCancel={() => setImportUrlPromptVisible(false)}
        testID="import-url-dialog"
      />

      {/* Import confirm sheet — what the server parsed, before anything is written */}
      <Modal
        visible={importPreview !== null}
        animationType="slide"
        transparent
        onRequestClose={cancelImport}
      >
        <View className="flex-1 bg-black/60 justify-end">
          <View
            className="bg-background-primary rounded-t-2xl max-h-[85%]"
            style={{ padding: spacing.md }}
            testID="import-preview-sheet"
          >
            <Text className="text-lg font-bold text-text-primary mb-1">Import Coach</Text>
            <Text className="text-xs text-text-tertiary mb-3" numberOfLines={1}>
              {pendingImport?.source}
            </Text>

            <ScrollView style={{ maxHeight: 340 }} testID="import-preview-body">
              {importPreview?.valid === false ? (
                <View testID="import-preview-invalid">
                  <Text className="text-error text-sm font-semibold mb-2">
                    This document is not a valid coach.
                  </Text>
                  {(importPreview.errors ?? []).map((message) => (
                    <Text key={message} className="text-error text-sm mb-1">
                      • {message}
                    </Text>
                  ))}
                </View>
              ) : (
                <View testID="import-preview-parsed">
                  <Text className="text-text-primary text-base font-semibold">
                    {importPreview?.parsed?.title ?? importPreview?.parsed?.name}
                  </Text>
                  <Text className="text-text-secondary text-sm mt-1">
                    {importPreview?.parsed?.purpose}
                  </Text>
                  <Text className="text-text-tertiary text-xs mt-2">
                    {importPreview?.parsed?.category}
                    {importPreview?.token_count !== undefined
                      ? ` · ${importPreview.token_count} tokens`
                      : ''}
                  </Text>
                  {(importPreview?.parsed?.tags ?? []).length > 0 && (
                    <Text className="text-text-tertiary text-xs mt-1">
                      {(importPreview?.parsed?.tags ?? []).join(', ')}
                    </Text>
                  )}
                  {importPreview?.duplicate_exists && (
                    <Text className="text-warning text-sm mt-3" testID="import-duplicate-warning">
                      You already have an identical coach. Importing adds a second copy.
                    </Text>
                  )}
                  {(importPreview?.warnings ?? []).map((message) => (
                    <Text key={message} className="text-warning text-sm mt-2">
                      • {message}
                    </Text>
                  ))}
                </View>
              )}
            </ScrollView>

            <View className="flex-row gap-3 mt-4">
              <TouchableOpacity
                className="flex-1 py-3 rounded-full items-center border border-border-subtle"
                onPress={cancelImport}
                testID="cancel-import-button"
              >
                <Text className="text-base font-semibold text-text-primary">Cancel</Text>
              </TouchableOpacity>
              <TouchableOpacity
                className="flex-1 py-3 rounded-full items-center"
                style={{
                  backgroundColor: importPreview?.valid
                    ? colors.pierre.violet
                    : colors.background.tertiary,
                }}
                onPress={() => void handleConfirmImport()}
                disabled={!importPreview?.valid || isImporting}
                testID="confirm-import-button"
              >
                {isImporting ? (
                  <ActivityIndicator size="small" color={colors.tokens.onPrimary} />
                ) : (
                  <Text
                    className="text-base font-semibold"
                    style={{ color: colors.tokens.onPrimary }}
                  >
                    Import
                  </Text>
                )}
              </TouchableOpacity>
            </View>
          </View>
        </View>
      </Modal>
    </SafeAreaView>
  );
}
