// ABOUTME: User Coach Library UI component for managing personal AI coaching personas
// ABOUTME: Provides CRUD operations for user-created coaches with category filtering and favorites
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState, useEffect, useRef, useCallback } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { BookOpen } from 'lucide-react';
import { coachesApi } from '../services/api';
import type { Coach } from '../types/api';
import type { ImportPreviewResponse } from '@pierre/shared-types';
import { Card, Button, TabHeader } from './ui';
import { clsx } from 'clsx';
import { QUERY_KEYS } from '../constants/queryKeys';

// Coach category options
const COACH_CATEGORIES = ['Training', 'Nutrition', 'Recovery', 'Recipes', 'Mobility', 'Custom'];

// Source filter options (user-created vs system coaches)
type CoachSource = 'all' | 'user' | 'system';
const SOURCE_FILTERS: Array<{ key: CoachSource; label: string }> = [
  { key: 'all', label: 'All Sources' },
  { key: 'user', label: 'My Coaches' },
  { key: 'system', label: 'System' },
];

// Category emoji icons matching mobile
const CATEGORY_EMOJIS: Record<string, string> = {
  Training: '🏃',
  Nutrition: '🥗',
  Recovery: '😴',
  Recipes: '👨‍🍳',
  Mobility: '🧘',
  Custom: '⚙️',
};

// Category colors for visual differentiation (matching ASY-35 specs)
const CATEGORY_COLORS: Record<string, string> = {
  Training: 'bg-pierre-activity/10 text-pierre-activity border-pierre-activity/20',
  Nutrition: 'bg-pierre-nutrition/10 text-pierre-nutrition border-pierre-nutrition/20',
  Recovery: 'bg-pierre-recovery/10 text-pierre-recovery border-pierre-recovery/20',
  Recipes: 'bg-pierre-yellow-500/10 text-pierre-yellow-600 border-pierre-yellow-500/20',
  Mobility: 'bg-pierre-mobility/10 text-pierre-mobility border-pierre-mobility/20',
  Custom: 'bg-pierre-violet/10 text-pierre-violet-light border-pierre-violet/20',
};

// Category border colors for left accent (synced with shared-constants)
const CATEGORY_BORDER_COLORS: Record<string, string> = {
  Training: '#3c6658',
  Nutrition: '#8f6a2e',
  Recovery: '#5e7a82',
  Recipes: '#F97316',
  Mobility: '#7a4d5e',
  Custom: '#00241a',
};

// LLM context window size for percentage calculation
const CONTEXT_WINDOW_SIZE = 128000;

interface CoachFormData {
  title: string;
  description: string;
  system_prompt: string;
  category: string;
  tags: string;
  startup_query: string;
  prefetch_enabled: boolean;
  activity_count: number;
  sport_types: string;
  time_frame: string;
  detail_mode: string;
  athlete_profile: boolean;
  purpose: string;
  when_to_use: string;
  instructions: string;
  example_inputs: string;
  example_outputs: string;
  success_criteria: string;
}

const defaultFormData: CoachFormData = {
  title: '',
  description: '',
  system_prompt: '',
  category: 'Training',
  tags: '',
  startup_query: '',
  prefetch_enabled: false,
  activity_count: 20,
  sport_types: '',
  time_frame: '12w',
  detail_mode: 'summary',
  athlete_profile: false,
  purpose: '',
  when_to_use: '',
  instructions: '',
  example_inputs: '',
  example_outputs: '',
  success_criteria: '',
};

interface CoachLibraryTabProps {
  onBack?: () => void;
}

export default function CoachLibraryTab({ onBack }: CoachLibraryTabProps) {
  const queryClient = useQueryClient();
  const [selectedCoach, setSelectedCoach] = useState<Coach | null>(null);
  const [isEditing, setIsEditing] = useState(false);
  const [isCreating, setIsCreating] = useState(false);
  const [formData, setFormData] = useState<CoachFormData>(defaultFormData);
  const [categoryFilter, setCategoryFilter] = useState<string | null>(null);
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const [selectedSource, setSelectedSource] = useState<CoachSource>('all');
  const [showHidden, setShowHidden] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');
  const [actionMenuCoach, setActionMenuCoach] = useState<Coach | null>(null);
  const [isRenameDialogOpen, setIsRenameDialogOpen] = useState(false);
  const [renameValue, setRenameValue] = useState('');
  const [showImportMenu, setShowImportMenu] = useState(false);
  const [showUrlDialog, setShowUrlDialog] = useState(false);
  const [importUrl, setImportUrl] = useState('');
  const [importPreviewData, setImportPreviewData] = useState<ImportPreviewResponse | null>(null);
  const [pendingImportSource, setPendingImportSource] = useState<{ type: 'file'; content: string } | { type: 'url'; url: string } | null>(null);
  const [importError, setImportError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const importMenuRef = useRef<HTMLDivElement>(null);

  // Fetch all coaches (including hidden) for client-side filtering
  const { data: coachesData, isLoading: coachesLoading } = useQuery({
    queryKey: QUERY_KEYS.coaches.listWithHidden(),
    queryFn: () => coachesApi.list({
      include_hidden: true,
    }),
  });

  // Fetch hidden coaches list to mark them
  const { data: hiddenData } = useQuery({
    queryKey: QUERY_KEYS.coaches.hidden(),
    queryFn: () => coachesApi.getHidden(),
  });

  // Create mutation
  const createMutation = useMutation({
    mutationFn: (data: typeof formData) => coachesApi.create({
      title: data.title,
      description: data.description || undefined,
      system_prompt: data.system_prompt,
      category: data.category,
      tags: data.tags.split(',').map(t => t.trim()).filter(Boolean),
      startup_query: data.startup_query.trim() || undefined,
      data_requirements: data.prefetch_enabled ? {
        activities: {
          count: data.activity_count,
          sport_types: data.sport_types.split(',').map(s => s.trim()).filter(Boolean),
          time_frame: data.time_frame,
          mode: data.detail_mode as 'summary' | 'detailed',
          format: 'toon' as const,
          analysis_type: 'general_overview',
        },
        athlete_profile: data.athlete_profile,
      } : undefined,
      purpose: data.purpose.trim() || undefined,
      when_to_use: data.when_to_use.trim() || undefined,
      instructions: data.instructions.trim() || undefined,
      example_inputs: data.example_inputs.trim() || undefined,
      example_outputs: data.example_outputs.trim() || undefined,
      success_criteria: data.success_criteria.trim() || undefined,
    }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      setIsCreating(false);
      setFormData(defaultFormData);
    },
  });

  // Update mutation
  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: typeof formData }) => coachesApi.update(id, {
      title: data.title,
      description: data.description || undefined,
      system_prompt: data.system_prompt,
      category: data.category,
      tags: data.tags.split(',').map(t => t.trim()).filter(Boolean),
      startup_query: data.startup_query.trim() || undefined,
      data_requirements: data.prefetch_enabled ? {
        activities: {
          count: data.activity_count,
          sport_types: data.sport_types.split(',').map(s => s.trim()).filter(Boolean),
          time_frame: data.time_frame,
          mode: data.detail_mode as 'summary' | 'detailed',
          format: 'toon' as const,
          analysis_type: 'general_overview',
        },
        athlete_profile: data.athlete_profile,
      } : undefined,
      purpose: data.purpose.trim() || undefined,
      when_to_use: data.when_to_use.trim() || undefined,
      instructions: data.instructions.trim() || undefined,
      example_inputs: data.example_inputs.trim() || undefined,
      example_outputs: data.example_outputs.trim() || undefined,
      success_criteria: data.success_criteria.trim() || undefined,
    }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      setIsEditing(false);
      setSelectedCoach(null);
    },
  });

  // Delete mutation
  const deleteMutation = useMutation({
    mutationFn: (id: string) => coachesApi.delete(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      setSelectedCoach(null);
    },
  });

  // Toggle favorite mutation
  const favoriteMutation = useMutation({
    mutationFn: (id: string) => coachesApi.toggleFavorite(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
    },
  });

  // Hide coach mutation
  const hideMutation = useMutation({
    mutationFn: (id: string) => coachesApi.hide(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.hidden() });
      setActionMenuCoach(null);
    },
  });

  // Show coach mutation
  const showMutation = useMutation({
    mutationFn: (id: string) => coachesApi.show(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.hidden() });
      setActionMenuCoach(null);
    },
  });

  // Fork coach mutation
  const forkMutation = useMutation({
    mutationFn: (id: string) => coachesApi.fork(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      setActionMenuCoach(null);
    },
  });

  // Import preview mutation (validates markdown before saving)
  const importPreviewMutation = useMutation({
    mutationFn: (markdown: string) => coachesApi.importPreview(markdown),
    onSuccess: (data) => {
      setImportPreviewData(data);
      setImportError(null);
    },
    onError: (error: Error) => {
      setImportError(error.message || 'Failed to preview import');
      setImportPreviewData(null);
    },
  });

  // Import from markdown mutation (saves the coach)
  const importMarkdownMutation = useMutation({
    mutationFn: (markdown: string) => coachesApi.importFromMarkdown(markdown),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      setImportPreviewData(null);
      setPendingImportSource(null);
      setImportError(null);
    },
    onError: (error: Error) => {
      setImportError(error.message || 'Failed to import coach');
    },
  });

  // Import from URL preview mutation
  const importUrlPreviewMutation = useMutation({
    mutationFn: (url: string) => coachesApi.importFromUrl(url, false),
    onSuccess: (data) => {
      setImportPreviewData(data as ImportPreviewResponse);
      setShowUrlDialog(false);
      setImportError(null);
    },
    onError: (error: Error) => {
      setImportError(error.message || 'Failed to fetch URL');
    },
  });

  // Import from URL save mutation
  const importUrlSaveMutation = useMutation({
    mutationFn: (url: string) => coachesApi.importFromUrl(url, true),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      setImportPreviewData(null);
      setPendingImportSource(null);
      setImportError(null);
    },
    onError: (error: Error) => {
      setImportError(error.message || 'Failed to import coach from URL');
    },
  });

  // Export coach mutation
  const exportMutation = useMutation({
    mutationFn: (coachId: string) => coachesApi.exportAsMarkdown(coachId),
    onSuccess: (markdown, coachId) => {
      const coach = (coachesData?.coaches || []).find(c => c.id === coachId);
      const filename = (coach?.title || 'coach').toLowerCase().replace(/\s+/g, '-') + '.md';
      const blob = new Blob([markdown], { type: 'text/markdown' });
      const url = URL.createObjectURL(blob);
      const link = document.createElement('a');
      link.href = url;
      link.download = filename;
      document.body.appendChild(link);
      link.click();
      document.body.removeChild(link);
      URL.revokeObjectURL(url);
    },
  });

  // Close import menu when clicking outside
  useEffect(() => {
    if (!showImportMenu) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (importMenuRef.current && !importMenuRef.current.contains(e.target as Node)) {
        setShowImportMenu(false);
      }
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, [showImportMenu]);

  // Handle file selection for import
  const handleFileImport = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = (event) => {
      const content = event.target?.result as string;
      setPendingImportSource({ type: 'file', content });
      importPreviewMutation.mutate(content);
    };
    reader.readAsText(file);
    // Reset file input so the same file can be selected again
    e.target.value = '';
  }, [importPreviewMutation]);

  // Handle URL import submission
  const handleUrlImportSubmit = useCallback(() => {
    const trimmed = importUrl.trim();
    if (!trimmed) return;
    setPendingImportSource({ type: 'url', url: trimmed });
    importUrlPreviewMutation.mutate(trimmed);
  }, [importUrl, importUrlPreviewMutation]);

  // Confirm import after preview
  const handleConfirmImport = useCallback(() => {
    if (!pendingImportSource) return;
    if (pendingImportSource.type === 'file') {
      importMarkdownMutation.mutate(pendingImportSource.content);
    } else {
      importUrlSaveMutation.mutate(pendingImportSource.url);
    }
  }, [pendingImportSource, importMarkdownMutation, importUrlSaveMutation]);

  // Cancel import preview
  const handleCancelImport = useCallback(() => {
    setImportPreviewData(null);
    setPendingImportSource(null);
    setImportError(null);
  }, []);

  // Load form data when editing
  useEffect(() => {
    if (isEditing && selectedCoach) {
      setFormData({
        title: selectedCoach.title,
        description: selectedCoach.description || '',
        system_prompt: selectedCoach.system_prompt,
        category: selectedCoach.category,
        tags: selectedCoach.tags.join(', '),
        startup_query: selectedCoach.startup_query || '',
        prefetch_enabled: !!selectedCoach.data_requirements?.activities,
        activity_count: selectedCoach.data_requirements?.activities?.count ?? 20,
        sport_types: (selectedCoach.data_requirements?.activities?.sport_types || []).join(', '),
        time_frame: selectedCoach.data_requirements?.activities?.time_frame || '12w',
        detail_mode: selectedCoach.data_requirements?.activities?.mode || 'summary',
        athlete_profile: selectedCoach.data_requirements?.athlete_profile ?? false,
        purpose: selectedCoach.purpose || '',
        when_to_use: selectedCoach.when_to_use || '',
        instructions: selectedCoach.instructions || '',
        example_inputs: selectedCoach.example_inputs || '',
        example_outputs: selectedCoach.example_outputs || '',
        success_criteria: selectedCoach.success_criteria || '',
      });
    }
  }, [isEditing, selectedCoach]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (isCreating) {
      createMutation.mutate(formData);
    } else if (isEditing && selectedCoach) {
      updateMutation.mutate({ id: selectedCoach.id, data: formData });
    }
  };

  const handleDelete = () => {
    if (selectedCoach && confirm(`Delete coach "${selectedCoach.title}"? This cannot be undone.`)) {
      deleteMutation.mutate(selectedCoach.id);
    }
  };

  const handleToggleFavorite = (e: React.MouseEvent, coachId: string) => {
    e.stopPropagation();
    favoriteMutation.mutate(coachId);
  };

  const handleHideCoach = (coach: Coach) => {
    hideMutation.mutate(coach.id);
  };

  const handleShowCoach = (coach: Coach) => {
    showMutation.mutate(coach.id);
  };

  const handleForkCoach = (coach: Coach) => {
    if (!coach.is_system) return;
    if (confirm(`Create your own copy of "${coach.title}"? You can customize the forked coach however you like.`)) {
      forkMutation.mutate(coach.id);
    }
  };

  const handleRename = (coach: Coach) => {
    setRenameValue(coach.title);
    setIsRenameDialogOpen(true);
  };

  const handleRenameSubmit = () => {
    if (actionMenuCoach && renameValue.trim()) {
      updateMutation.mutate({
        id: actionMenuCoach.id,
        data: { ...formData, title: renameValue.trim() },
      });
      setIsRenameDialogOpen(false);
      setActionMenuCoach(null);
    }
  };

  const handleContextMenu = (e: React.MouseEvent, coach: Coach) => {
    e.preventDefault();
    e.stopPropagation();
    setActionMenuCoach(coach);
  };

  const closeActionMenu = () => {
    setActionMenuCoach(null);
  };

  // Build set of hidden coach IDs for quick lookup
  const hiddenIds = new Set((hiddenData?.coaches || []).map(c => c.id));

  // Apply client-side filtering based on all filter states
  const filteredCoaches = (coachesData?.coaches || [])
    // Mark coaches as hidden
    .map(coach => ({ ...coach, is_hidden: hiddenIds.has(coach.id) }))
    // Filter by hidden state
    .filter(coach => showHidden || !coach.is_hidden)
    // Filter by source (user vs system)
    .filter(coach => {
      if (selectedSource === 'user') return !coach.is_system;
      if (selectedSource === 'system') return coach.is_system;
      return true;
    })
    // Filter by category
    .filter(coach => !categoryFilter || coach.category === categoryFilter)
    // Filter by favorites
    .filter(coach => !favoritesOnly || coach.is_favorite)
    // Filter by search query
    .filter(coach => {
      if (!searchQuery.trim()) return true;
      const query = searchQuery.toLowerCase();
      return (
        coach.title.toLowerCase().includes(query) ||
        (coach.description || '').toLowerCase().includes(query)
      );
    })
    // Sort: favorites first, then by use_count
    .sort((a, b) => {
      if (a.is_favorite !== b.is_favorite) return a.is_favorite ? -1 : 1;
      return b.use_count - a.use_count;
    });

  // Token count estimation (same formula as mobile: text.length / 4)
  const estimateTokenCount = (text: string): number => {
    return Math.ceil(text.length / 4);
  };

  // Context percentage calculation (tokens / 128000 * 100)
  const getContextPercentage = (tokens: number): string => {
    return ((tokens / CONTEXT_WINDOW_SIZE) * 100).toFixed(1);
  };

  // Coach list view
  if (!selectedCoach && !isCreating) {
    return (
      <div className="h-full flex flex-col bg-surface">
        <TabHeader
          icon={<BookOpen className="w-5 h-5" />}
          gradient="from-pierre-cyan to-pierre-blue-600"
          description="Create custom AI personas to get specialized fitness coaching."
          actions={
            <>
              {onBack && (
                <button
                  onClick={onBack}
                  className="p-2 rounded-lg text-on-surface-variant hover:text-pierre-violet hover:bg-surface-container-low transition-colors min-w-[44px] min-h-[44px] flex items-center justify-center"
                  title="Back"
                  aria-label="Back"
                >
                  <svg className="w-4 h-4" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
                  </svg>
                </button>
              )}
              <button
                onClick={() => setShowHidden(!showHidden)}
                className={clsx(
                  'p-2 rounded-lg transition-colors min-w-[44px] min-h-[44px] flex items-center justify-center',
                  showHidden
                    ? 'bg-pierre-violet/20 text-pierre-violet-light'
                    : 'text-outline hover:text-on-surface hover:bg-surface-container-low'
                )}
                title={showHidden ? 'Hide hidden coaches' : 'Show hidden coaches'}
                aria-label={showHidden ? 'Hide hidden coaches' : 'Show hidden coaches'}
              >
                <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  {showHidden ? (
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                  ) : (
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21" />
                  )}
                </svg>
              </button>
              {/* Import button with dropdown */}
              <div className="relative" ref={importMenuRef}>
                <button
                  onClick={() => setShowImportMenu(!showImportMenu)}
                  className="p-2 rounded-lg text-on-surface-variant hover:text-pierre-violet hover:bg-surface-container-low transition-colors min-w-[44px] min-h-[44px] flex items-center justify-center"
                  title="Import Coach"
                  aria-label="Import Coach"
                >
                  <svg className="w-4 h-4" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12" />
                  </svg>
                </button>
                {showImportMenu && (
                  <div className="absolute right-0 top-full mt-1 w-48 bg-[#1E1B2D] rounded-lg border ghost-border shadow-xl z-50 py-1">
                    <button
                      onClick={() => {
                        setShowImportMenu(false);
                        fileInputRef.current?.click();
                      }}
                      className="w-full flex items-center gap-2 px-3 py-2 text-sm text-on-surface hover:bg-surface-container-low transition-colors"
                    >
                      <svg className="w-4 h-4" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
                      </svg>
                      Import from File
                    </button>
                    <button
                      onClick={() => {
                        setShowImportMenu(false);
                        setImportUrl('');
                        setImportError(null);
                        setShowUrlDialog(true);
                      }}
                      className="w-full flex items-center gap-2 px-3 py-2 text-sm text-on-surface hover:bg-surface-container-low transition-colors"
                    >
                      <svg className="w-4 h-4" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
                      </svg>
                      Import from URL
                    </button>
                  </div>
                )}
              </div>
              {/* Hidden file input for markdown import */}
              <input
                ref={fileInputRef}
                type="file"
                accept=".md,text/markdown"
                className="hidden"
                onChange={handleFileImport}
                aria-hidden="true"
              />
              <button
                onClick={() => {
                  setFormData(defaultFormData);
                  setIsCreating(true);
                }}
                className="p-2 rounded-lg text-on-surface bg-pierre-violet hover:bg-pierre-violet-dark transition-colors shadow-ambient hover:shadow-ambient min-w-[44px] min-h-[44px] flex items-center justify-center"
                title="Create Coach"
                aria-label="Create Coach"
              >
                <svg className="w-4 h-4" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
                </svg>
              </button>
            </>
          }
        />

        {/* Search Bar */}
        <div className="px-6 py-4 border-b ghost-border">
          <div className="relative">
            <svg
              className="absolute left-3 top-1/2 transform -translate-y-1/2 w-5 h-5 text-outline"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
              aria-hidden="true"
            >
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
            </svg>
            <input
              type="search"
              placeholder="Search coaches..."
              aria-label="Search coaches"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="w-full pl-10 pr-10 py-2.5 bg-surface-container-low border ghost-border rounded-lg text-sm text-on-surface placeholder:text-outline focus:outline-none focus:ring-2 focus:ring-pierre-violet/30 focus:border-pierre-violet transition-colors"
            />
            {searchQuery && (
              <button
                onClick={() => setSearchQuery('')}
                aria-label="Clear search"
                className="absolute right-1 top-1/2 transform -translate-y-1/2 text-outline hover:text-on-surface min-w-[44px] min-h-[44px] flex items-center justify-center"
              >
                <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            )}
          </div>
        </div>

        {/* Category Filters - inline with Favorites */}
        <div className="px-6 py-3 border-b ghost-border overflow-x-auto">
          <div className="flex items-center gap-2">
            <button
              onClick={() => setCategoryFilter(null)}
              className={clsx(
                'px-4 py-1.5 text-sm font-medium rounded-full whitespace-nowrap transition-colors min-h-[44px] flex items-center',
                categoryFilter === null
                  ? 'bg-pierre-violet text-on-surface shadow-ambient'
                  : 'bg-surface-container-low text-on-surface-variant hover:bg-surface-container hover:text-on-surface'
              )}
            >
              All
            </button>
            {COACH_CATEGORIES.map((cat) => (
              <button
                key={cat}
                onClick={() => setCategoryFilter(cat)}
                className={clsx(
                  'px-4 py-1.5 text-sm font-medium rounded-full whitespace-nowrap transition-colors min-h-[44px] flex items-center',
                  categoryFilter === cat
                    ? 'bg-pierre-violet text-on-surface shadow-ambient'
                    : 'bg-surface-container-low text-on-surface-variant hover:bg-surface-container hover:text-on-surface'
                )}
              >
                {cat}
              </button>
            ))}
            {/* Favorites toggle - inline with categories */}
            <button
              onClick={() => setFavoritesOnly(!favoritesOnly)}
              className={clsx(
                'flex items-center gap-1 px-4 py-1.5 text-sm font-medium rounded-full whitespace-nowrap transition-colors min-h-[44px]',
                favoritesOnly
                  ? 'bg-pierre-yellow-500/20 text-pierre-yellow-400'
                  : 'bg-surface-container-low text-on-surface-variant hover:bg-surface-container hover:text-on-surface'
              )}
            >
              <svg
                className={clsx('w-4 h-4', favoritesOnly ? 'fill-pierre-yellow-500' : 'fill-none')}
                stroke="currentColor"
                viewBox="0 0 24 24"
                aria-hidden="true"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M11.049 2.927c.3-.921 1.603-.921 1.902 0l1.519 4.674a1 1 0 00.95.69h4.915c.969 0 1.371 1.24.588 1.81l-3.976 2.888a1 1 0 00-.363 1.118l1.518 4.674c.3.922-.755 1.688-1.538 1.118l-3.976-2.888a1 1 0 00-1.176 0l-3.976 2.888c-.783.57-1.838-.197-1.538-1.118l1.518-4.674a1 1 0 00-.363-1.118l-3.976-2.888c-.784-.57-.38-1.81.588-1.81h4.914a1 1 0 00.951-.69l1.519-4.674z"
                />
              </svg>
              Favorites
            </button>
          </div>
        </div>

        {/* Source filter (All Sources / My Coaches / System) */}
        <div className="px-6 py-2 bg-surface-container-low border-b ghost-border flex justify-center items-center gap-3">
          {SOURCE_FILTERS.map((filter) => (
            <button
              key={filter.key}
              onClick={() => setSelectedSource(filter.key)}
              className={clsx(
                'px-3 py-1 text-sm font-medium rounded transition-colors min-h-[44px] flex items-center',
                selectedSource === filter.key
                  ? 'bg-pierre-violet/20 text-pierre-violet-light font-medium'
                  : 'text-on-surface-variant hover:text-pierre-violet-light'
              )}
            >
              {filter.label}
            </button>
          ))}
        </div>

        {/* Coaches Grid - scrollable content area */}
        <div className="flex-1 overflow-y-auto p-6">
        {coachesLoading ? (
          <div className="flex justify-center py-12">
            <div className="pierre-spinner w-8 h-8"></div>
          </div>
        ) : filteredCoaches.length === 0 ? (
          <Card variant="dark" className="text-center py-12">
            <div className="w-16 h-16 mx-auto mb-4 rounded-full bg-surface-container-low flex items-center justify-center">
              <svg className="w-8 h-8 text-outline" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5.121 17.804A13.937 13.937 0 0112 16c2.5 0 4.847.655 6.879 1.804M15 10a3 3 0 11-6 0 3 3 0 016 0zm6 2a9 9 0 11-18 0 9 9 0 0118 0z" />
              </svg>
            </div>
            <h3 className="text-lg font-medium text-on-surface mb-2">
              {favoritesOnly ? 'No Favorite Coaches' :
               selectedSource === 'user' ? 'No User-Created Coaches' :
               selectedSource === 'system' ? 'No System Coaches' :
               categoryFilter ? `No ${categoryFilter} Coaches` :
               'No Coaches Yet'}
            </h3>
            <p className="text-on-surface-variant mb-4">
              {favoritesOnly
                ? 'Star some coaches to see them here.'
                : (coachesData?.coaches || []).length === 0
                ? 'Create your first coach to customize how Pierre helps you.'
                : 'Try adjusting your filters.'}
            </p>
            {!favoritesOnly && (coachesData?.coaches || []).length === 0 && (
              <Button onClick={() => setIsCreating(true)}>Create Your First Coach</Button>
            )}
          </Card>
        ) : (
          <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
            {filteredCoaches.map((coach) => {
              const isHidden = coach.is_hidden;
              return (
                <div
                  key={coach.id}
                  className={clsx(
                    'cursor-pointer hover:shadow-md transition-all border-l-4 bg-surface-container-low rounded-xl p-4 border ghost-border',
                    isHidden && 'opacity-60'
                  )}
                  style={{ borderLeftColor: CATEGORY_BORDER_COLORS[coach.category] || CATEGORY_BORDER_COLORS.Custom }}
                  onClick={() => setSelectedCoach(coach)}
                  onContextMenu={(e) => handleContextMenu(e, coach)}
                >
                  <div className="flex items-start gap-3">
                    {/* Category Emoji Avatar */}
                    <div
                      className="w-12 h-12 rounded-xl flex items-center justify-center flex-shrink-0 text-xl"
                      style={{ backgroundColor: `${CATEGORY_BORDER_COLORS[coach.category] || CATEGORY_BORDER_COLORS.Custom}20` }}
                    >
                      {CATEGORY_EMOJIS[coach.category] || CATEGORY_EMOJIS.Custom}
                    </div>

                    <div className="flex-1 min-w-0">
                      {/* Title and badges */}
                      <div className="flex items-center gap-2 mb-1">
                        <h3 className={clsx('font-semibold', isHidden ? 'text-outline' : 'text-on-surface')}>
                          {coach.title}
                        </h3>
                        <span className={clsx(
                          'px-2 py-0.5 text-xs font-medium rounded-full border flex-shrink-0',
                          CATEGORY_COLORS[coach.category] || CATEGORY_COLORS.Custom
                        )}>
                          {coach.category}
                        </span>
                        {coach.is_system && (
                          <span className="px-2 py-0.5 text-xs font-medium rounded-full bg-zinc-700/50 text-on-surface-variant flex-shrink-0">
                            System
                          </span>
                        )}
                      </div>

                      {/* Star rating (use count as proxy) and favorite button */}
                      <div className="flex items-center gap-1 mb-1">
                        {[1, 2, 3, 4, 5].map((star) => (
                          <svg
                            key={star}
                            className={clsx(
                              'w-3 h-3',
                              coach.use_count >= star * 2 ? 'text-pierre-yellow-500 fill-pierre-yellow-500' : 'text-on-surface-variant fill-none'
                            )}
                            stroke="currentColor"
                            viewBox="0 0 24 24"
                            aria-hidden="true"
                          >
                            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M11.049 2.927c.3-.921 1.603-.921 1.902 0l1.519 4.674a1 1 0 00.95.69h4.915c.969 0 1.371 1.24.588 1.81l-3.976 2.888a1 1 0 00-.363 1.118l1.518 4.674c.3.922-.755 1.688-1.538 1.118l-3.976-2.888a1 1 0 00-1.176 0l-3.976 2.888c-.783.57-1.838-.197-1.538-1.118l1.518-4.674a1 1 0 00-.363-1.118l-3.976-2.888c-.784-.57-.38-1.81.588-1.81h4.914a1 1 0 00.951-.69l1.519-4.674z" />
                          </svg>
                        ))}
                        <button
                          onClick={(e) => handleToggleFavorite(e, coach.id)}
                          className="ml-2 p-2 min-w-[44px] min-h-[44px] flex items-center justify-center text-outline hover:text-pierre-violet transition-colors"
                          title={coach.is_favorite ? 'Remove from favorites' : 'Add to favorites'}
                        >
                          <svg className="w-4 h-4" aria-hidden="true" fill={coach.is_favorite ? 'currentColor' : 'none'} stroke="currentColor" viewBox="0 0 24 24">
                            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4.318 6.318a4.5 4.5 0 000 6.364L12 20.364l7.682-7.682a4.5 4.5 0 00-6.364-6.364L12 7.636l-1.318-1.318a4.5 4.5 0 00-6.364 0z" />
                          </svg>
                        </button>
                      </div>

                      {/* Description */}
                      {coach.description && (
                        <p className={clsx('text-sm line-clamp-4', isHidden ? 'text-on-surface-variant' : 'text-on-surface-variant')}>
                          {coach.description}
                        </p>
                      )}
                    </div>

                    {/* Chat button with violet glow */}
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        setSelectedCoach(coach);
                      }}
                      className="px-4 py-2 rounded-full text-sm font-semibold text-on-surface flex-shrink-0"
                      style={{
                        backgroundColor: '#00241a',
                        boxShadow: '0 0 12px rgba(0, 36, 26, 0.4)',
                      }}
                    >
                      Chat
                    </button>
                  </div>

                  {/* Action row with export (always) and system/hidden actions */}
                  <div className="flex items-center justify-end mt-3 pt-2 border-t ghost-border gap-2">
                    {/* Export button (available for all coaches) */}
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        exportMutation.mutate(coach.id);
                      }}
                      disabled={exportMutation.isPending}
                      className="flex items-center gap-1 px-2 py-1 rounded text-xs text-outline hover:text-on-surface bg-surface-container-low hover:bg-surface-container transition-colors"
                    >
                      <svg className="w-3.5 h-3.5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
                      </svg>
                      {exportMutation.isPending ? 'Exporting...' : 'Export'}
                    </button>
                    {/* Fork button for system coaches */}
                    {coach.is_system && (
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          handleForkCoach(coach);
                        }}
                        className="flex items-center gap-1 px-2 py-1 rounded text-xs text-outline hover:text-on-surface bg-surface-container-low hover:bg-surface-container transition-colors"
                      >
                        <svg className="w-3.5 h-3.5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z" />
                        </svg>
                        Fork
                      </button>
                    )}
                    {/* Hide/Show button */}
                    {coach.is_system && (
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          if (isHidden) {
                            handleShowCoach(coach);
                          } else {
                            handleHideCoach(coach);
                          }
                        }}
                        className="flex items-center gap-1 px-2 py-1 rounded text-xs text-outline hover:text-on-surface bg-surface-container-low hover:bg-surface-container transition-colors"
                      >
                        <svg className="w-3.5 h-3.5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          {isHidden ? (
                            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                          ) : (
                            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21" />
                          )}
                        </svg>
                        {isHidden ? 'Show' : 'Hide'}
                      </button>
                    )}
                    {/* Hidden indicator for non-system coaches */}
                    {isHidden && !coach.is_system && (
                      <span className="flex items-center gap-1 text-xs text-outline">
                        <svg className="w-3.5 h-3.5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21" />
                        </svg>
                        Hidden
                      </span>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}
        </div>

        {/* Context Menu Modal */}
        {actionMenuCoach && (
          <div
            className="fixed inset-0 z-50 bg-black/30 flex items-center justify-center"
            onClick={closeActionMenu}
          >
            <div
              className="bg-[#1E1B2D] rounded-xl p-2 min-w-[220px] shadow-xl border ghost-border"
              onClick={(e) => e.stopPropagation()}
            >
              {/* Favorite toggle */}
              <button
                onClick={() => {
                  favoriteMutation.mutate(actionMenuCoach.id);
                  closeActionMenu();
                }}
                className="w-full flex items-center gap-3 px-3 py-2 text-left text-on-surface hover:bg-surface-container-low rounded-lg transition-colors"
              >
                <svg
                  className={clsx('w-5 h-5', actionMenuCoach.is_favorite ? 'text-pierre-yellow-500' : '')}
                  fill={actionMenuCoach.is_favorite ? 'currentColor' : 'none'}
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                  aria-hidden="true"
                >
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M11.049 2.927c.3-.921 1.603-.921 1.902 0l1.519 4.674a1 1 0 00.95.69h4.915c.969 0 1.371 1.24.588 1.81l-3.976 2.888a1 1 0 00-.363 1.118l1.518 4.674c.3.922-.755 1.688-1.538 1.118l-3.976-2.888a1 1 0 00-1.176 0l-3.976 2.888c-.783.57-1.838-.197-1.538-1.118l1.518-4.674a1 1 0 00-.363-1.118l-3.976-2.888c-.784-.57-.38-1.81.588-1.81h4.914a1 1 0 00.951-.69l1.519-4.674z" />
                </svg>
                {actionMenuCoach.is_favorite ? 'Remove from favorites' : 'Add to favorites'}
              </button>

              {/* Hide/Show for system coaches */}
              {actionMenuCoach.is_system && (
                <button
                  onClick={() => {
                    if (actionMenuCoach.is_hidden) {
                      handleShowCoach(actionMenuCoach);
                    } else {
                      handleHideCoach(actionMenuCoach);
                    }
                  }}
                  className="w-full flex items-center gap-3 px-3 py-2 text-left text-on-surface hover:bg-surface-container-low rounded-lg transition-colors"
                >
                  <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    {actionMenuCoach.is_hidden ? (
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                    ) : (
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21" />
                    )}
                  </svg>
                  {actionMenuCoach.is_hidden ? 'Show coach' : 'Hide coach'}
                </button>
              )}

              {/* Fork for system coaches */}
              {actionMenuCoach.is_system && (
                <button
                  onClick={() => handleForkCoach(actionMenuCoach)}
                  className="w-full flex items-center gap-3 px-3 py-2 text-left text-on-surface hover:bg-surface-container-low rounded-lg transition-colors"
                >
                  <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z" />
                  </svg>
                  Fork (create my copy)
                </button>
              )}

              {/* Rename for user coaches */}
              {!actionMenuCoach.is_system && (
                <button
                  onClick={() => handleRename(actionMenuCoach)}
                  className="w-full flex items-center gap-3 px-3 py-2 text-left text-on-surface hover:bg-surface-container-low rounded-lg transition-colors"
                >
                  <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z" />
                  </svg>
                  Rename
                </button>
              )}

              {/* Delete for user coaches */}
              {!actionMenuCoach.is_system && (
                <button
                  onClick={() => {
                    if (confirm(`Delete coach "${actionMenuCoach.title}"? This cannot be undone.`)) {
                      deleteMutation.mutate(actionMenuCoach.id);
                      closeActionMenu();
                    }
                  }}
                  className="w-full flex items-center gap-3 px-3 py-2 text-left text-pierre-red-500 hover:bg-surface-container-low rounded-lg transition-colors"
                >
                  <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                  </svg>
                  Delete
                </button>
              )}
            </div>
          </div>
        )}

        {/* Rename Dialog */}
        {isRenameDialogOpen && actionMenuCoach && (
          <div
            className="fixed inset-0 z-50 bg-black/50 flex items-center justify-center"
            onClick={() => {
              setIsRenameDialogOpen(false);
              setActionMenuCoach(null);
            }}
          >
            <div
              className="bg-[#1E1B2D] rounded-xl p-6 w-full max-w-md shadow-xl border ghost-border"
              onClick={(e) => e.stopPropagation()}
            >
              <h3 className="text-lg font-semibold text-on-surface mb-4">Rename Coach</h3>
              <input
                type="text"
                value={renameValue}
                onChange={(e) => setRenameValue(e.target.value)}
                className="w-full px-3 py-2 bg-surface-container-low border ghost-border rounded-lg text-on-surface placeholder:text-outline focus:ring-2 focus:ring-pierre-violet focus:border-transparent mb-4"
                placeholder="Enter new name"
                autoFocus
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    handleRenameSubmit();
                  }
                }}
              />
              <div className="flex justify-end gap-3">
                <Button
                  variant="secondary"
                  onClick={() => {
                    setIsRenameDialogOpen(false);
                    setActionMenuCoach(null);
                  }}
                >
                  Cancel
                </Button>
                <Button onClick={handleRenameSubmit}>Save</Button>
              </div>
            </div>
          </div>
        )}

        {/* Import from URL Dialog */}
        {showUrlDialog && (
          <div
            className="fixed inset-0 z-50 bg-black/50 flex items-center justify-center"
            onClick={() => {
              setShowUrlDialog(false);
              setImportError(null);
            }}
          >
            <div
              className="bg-[#1E1B2D] rounded-xl p-6 w-full max-w-md shadow-xl border ghost-border"
              onClick={(e) => e.stopPropagation()}
            >
              <h3 className="text-lg font-semibold text-on-surface mb-2">Import from URL</h3>
              <p className="text-sm text-on-surface-variant mb-4">Enter the URL of a markdown coach file.</p>
              <input
                type="url"
                value={importUrl}
                onChange={(e) => setImportUrl(e.target.value)}
                className="w-full px-3 py-2 bg-surface-container-low border ghost-border rounded-lg text-on-surface placeholder:text-outline focus:ring-2 focus:ring-pierre-violet focus:border-transparent mb-3"
                placeholder="https://example.com/coach.md"
                autoFocus
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    handleUrlImportSubmit();
                  }
                }}
              />
              {importError && (
                <p className="text-sm text-pierre-red-500 mb-3">{importError}</p>
              )}
              <div className="flex justify-end gap-3">
                <Button
                  variant="secondary"
                  onClick={() => {
                    setShowUrlDialog(false);
                    setImportError(null);
                  }}
                >
                  Cancel
                </Button>
                <Button
                  onClick={handleUrlImportSubmit}
                  disabled={!importUrl.trim() || importUrlPreviewMutation.isPending}
                >
                  {importUrlPreviewMutation.isPending ? (
                    <span className="flex items-center gap-2">
                      <div className="pierre-spinner w-4 h-4"></div>
                      Fetching...
                    </span>
                  ) : (
                    'Preview'
                  )}
                </Button>
              </div>
            </div>
          </div>
        )}

        {/* Import Preview Dialog */}
        {importPreviewData && (
          <div
            className="fixed inset-0 z-50 bg-black/50 flex items-center justify-center"
            onClick={handleCancelImport}
          >
            <div
              className="bg-[#1E1B2D] rounded-xl p-6 w-full max-w-lg shadow-xl border ghost-border"
              onClick={(e) => e.stopPropagation()}
            >
              <h3 className="text-lg font-semibold text-on-surface mb-4">Import Preview</h3>

              {!importPreviewData.valid ? (
                <div>
                  <p className="text-sm text-pierre-red-500 mb-3">This file cannot be imported:</p>
                  {importPreviewData.errors && importPreviewData.errors.length > 0 && (
                    <ul className="list-disc list-inside text-sm text-pierre-red-400 mb-4 space-y-1">
                      {importPreviewData.errors.map((err, i) => (
                        <li key={i}>{err}</li>
                      ))}
                    </ul>
                  )}
                  <div className="flex justify-end">
                    <Button variant="secondary" onClick={handleCancelImport}>Close</Button>
                  </div>
                </div>
              ) : (
                <div>
                  {importPreviewData.parsed && (
                    <div className="space-y-2 mb-4">
                      <div className="flex items-center gap-2">
                        <span className="text-sm text-on-surface-variant">Name:</span>
                        <span className="text-sm text-on-surface font-medium">{importPreviewData.parsed.title}</span>
                      </div>
                      <div className="flex items-center gap-2">
                        <span className="text-sm text-on-surface-variant">Category:</span>
                        <span className={clsx(
                          'px-2 py-0.5 text-xs font-medium rounded-full border',
                          CATEGORY_COLORS[importPreviewData.parsed.category] || CATEGORY_COLORS.Custom
                        )}>
                          {importPreviewData.parsed.category}
                        </span>
                      </div>
                      {importPreviewData.parsed.tags.length > 0 && (
                        <div className="flex items-center gap-2 flex-wrap">
                          <span className="text-sm text-on-surface-variant">Tags:</span>
                          {importPreviewData.parsed.tags.map((tag) => (
                            <span key={tag} className="px-2 py-0.5 text-xs bg-surface-container-low text-on-surface rounded-full">{tag}</span>
                          ))}
                        </div>
                      )}
                      {importPreviewData.token_count !== undefined && (
                        <div className="flex items-center gap-2">
                          <span className="text-sm text-on-surface-variant">Tokens:</span>
                          <span className="text-sm text-on-surface">~{importPreviewData.token_count.toLocaleString()}</span>
                        </div>
                      )}
                      <div className="flex items-center gap-3 text-xs text-outline">
                        <span>Purpose: {importPreviewData.parsed.purpose ? 'Yes' : 'No'}</span>
                        <span>Instructions: {importPreviewData.parsed.has_instructions ? 'Yes' : 'No'}</span>
                        <span>Examples: {importPreviewData.parsed.has_example_inputs ? 'Yes' : 'No'}</span>
                      </div>
                    </div>
                  )}

                  {importPreviewData.duplicate_exists && (
                    <div className="p-3 rounded-lg bg-pierre-yellow-500/10 border border-pierre-yellow-500/20 mb-4">
                      <p className="text-sm text-pierre-yellow-400">
                        A coach with matching content already exists. Importing will create a duplicate.
                      </p>
                    </div>
                  )}

                  {importPreviewData.warnings && importPreviewData.warnings.length > 0 && (
                    <div className="p-3 rounded-lg bg-pierre-yellow-500/10 border border-pierre-yellow-500/20 mb-4">
                      <p className="text-sm font-medium text-pierre-yellow-400 mb-1">Warnings:</p>
                      <ul className="list-disc list-inside text-sm text-pierre-yellow-300 space-y-1">
                        {importPreviewData.warnings.map((warn, i) => (
                          <li key={i}>{warn}</li>
                        ))}
                      </ul>
                    </div>
                  )}

                  {importError && (
                    <p className="text-sm text-pierre-red-500 mb-3">{importError}</p>
                  )}

                  <div className="flex justify-end gap-3">
                    <Button variant="secondary" onClick={handleCancelImport}>Cancel</Button>
                    <Button
                      onClick={handleConfirmImport}
                      disabled={importMarkdownMutation.isPending || importUrlSaveMutation.isPending}
                    >
                      {(importMarkdownMutation.isPending || importUrlSaveMutation.isPending) ? (
                        <span className="flex items-center gap-2">
                          <div className="pierre-spinner w-4 h-4"></div>
                          Importing...
                        </span>
                      ) : (
                        'Import'
                      )}
                    </Button>
                  </div>
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    );
  }

  // Create/Edit form view
  if (isCreating || isEditing) {
    const tokenCount = estimateTokenCount(formData.system_prompt);

    return (
      <div className="max-w-2xl mx-auto">
        <Card variant="dark">
          {/* Card header with integrated back button - industry standard pattern */}
          <div className="flex items-center gap-3 pb-4 mb-6 border-b ghost-border">
            <button
              onClick={() => {
                setIsCreating(false);
                setIsEditing(false);
                setFormData(defaultFormData);
                setSelectedCoach(null);
              }}
              className="p-1.5 rounded-lg text-outline hover:text-pierre-violet hover:bg-surface-container-low transition-colors"
              title="Back to coaches"
            >
              <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
              </svg>
            </button>
            <h2 className="text-xl font-semibold text-on-surface">
              {isCreating ? 'Create Coach' : `Edit "${selectedCoach?.title}"`}
            </h2>
          </div>

          <form onSubmit={handleSubmit} className="space-y-6">
            {/* Title */}
            <div>
              <label className="block text-sm font-medium text-on-surface mb-1">
                Title <span className="text-pierre-red-500">*</span>
              </label>
              <input
                type="text"
                value={formData.title}
                onChange={(e) => setFormData({ ...formData, title: e.target.value })}
                className="w-full px-3 py-2 bg-surface-container-low border ghost-border rounded-lg text-on-surface placeholder:text-outline focus:ring-2 focus:ring-pierre-violet focus:border-transparent"
                placeholder="e.g., Marathon Training Coach"
                maxLength={100}
                required
              />
            </div>

            {/* Category */}
            <div>
              <label className="block text-sm font-medium text-on-surface mb-1">
                Category
              </label>
              <select
                value={formData.category}
                onChange={(e) => setFormData({ ...formData, category: e.target.value })}
                className="w-full px-3 py-2 bg-surface-container-low border ghost-border rounded-lg text-on-surface focus:ring-2 focus:ring-pierre-violet focus:border-transparent"
              >
                {COACH_CATEGORIES.map((cat) => (
                  <option key={cat} value={cat} className="bg-surface-container-low">{cat}</option>
                ))}
              </select>
            </div>

            {/* Description */}
            <div>
              <label className="block text-sm font-medium text-on-surface mb-1">
                Description
              </label>
              <textarea
                value={formData.description}
                onChange={(e) => setFormData({ ...formData, description: e.target.value })}
                className="w-full px-3 py-2 bg-surface-container-low border ghost-border rounded-lg text-on-surface placeholder:text-outline focus:ring-2 focus:ring-pierre-violet focus:border-transparent"
                rows={2}
                maxLength={500}
                placeholder="Brief description of the coach's specialty..."
              />
              <p className="mt-1 text-xs text-outline text-right">
                {formData.description.length}/500
              </p>
            </div>

            {/* System Prompt */}
            <div>
              <label className="block text-sm font-medium text-on-surface mb-1">
                System Prompt <span className="text-pierre-red-500">*</span>
              </label>
              <textarea
                value={formData.system_prompt}
                onChange={(e) => setFormData({ ...formData, system_prompt: e.target.value })}
                className="w-full px-3 py-2 bg-surface-container-low border ghost-border rounded-lg text-on-surface placeholder:text-outline focus:ring-2 focus:ring-pierre-violet focus:border-transparent font-mono text-sm"
                rows={8}
                maxLength={4000}
                placeholder="You are Pierre, an expert coach with deep knowledge of..."
                required
              />
              <div className="mt-1 flex items-center justify-between text-xs text-outline">
                <span>
                  ~{tokenCount.toLocaleString()} tokens ({getContextPercentage(tokenCount)}% context)
                </span>
                <span>{formData.system_prompt.length}/4000</span>
              </div>
            </div>

            {/* Tags */}
            <div>
              <label className="block text-sm font-medium text-on-surface mb-1">
                Tags
              </label>
              <input
                type="text"
                value={formData.tags}
                onChange={(e) => setFormData({ ...formData, tags: e.target.value })}
                className="w-full px-3 py-2 bg-surface-container-low border ghost-border rounded-lg text-on-surface placeholder:text-outline focus:ring-2 focus:ring-pierre-violet focus:border-transparent"
                placeholder="marathon, endurance, beginner (comma-separated)"
              />
            </div>

            {/* Data Context */}
            <div className="mb-6">
              <h3 className="text-sm font-semibold text-on-surface/90 mb-3">Data Context</h3>

              <div className="mb-4">
                <label className="block text-sm text-on-surface/60 mb-1">
                  Startup Query <span className="text-on-surface/30">(optional)</span>
                </label>
                <textarea
                  className="w-full p-3 bg-surface-container-low border ghost-border rounded-lg text-on-surface/90 text-sm placeholder-white/30 focus:border-purple-500/50 focus:outline-none resize-none"
                  rows={2}
                  placeholder="What should the coach analyze on first message?"
                  value={formData.startup_query}
                  onChange={(e) => setFormData({ ...formData, startup_query: e.target.value })}
                />
              </div>

              <label className="flex items-center gap-2 mb-3 cursor-pointer">
                <input
                  type="checkbox"
                  checked={formData.prefetch_enabled}
                  onChange={(e) => setFormData({ ...formData, prefetch_enabled: e.target.checked })}
                  className="w-4 h-4 rounded ghost-border text-purple-500 focus:ring-purple-500 bg-surface-container-low"
                />
                <span className="text-sm text-on-surface/70">Pre-fetch activity data when conversation starts</span>
              </label>

              {formData.prefetch_enabled && (
                <div className="space-y-3 pl-4 border-l-2 border-purple-500/20">
                  <div className="grid grid-cols-2 gap-3">
                    <div>
                      <label className="block text-xs text-on-surface/50 mb-1">Activity count</label>
                      <input
                        type="number"
                        min={1}
                        max={200}
                        value={formData.activity_count}
                        onChange={(e) => setFormData({ ...formData, activity_count: Math.max(1, Math.min(200, Number(e.target.value))) })}
                        className="w-full p-2 bg-surface-container-low border ghost-border rounded-lg text-on-surface/90 text-sm focus:border-purple-500/50 focus:outline-none"
                      />
                    </div>
                    <div>
                      <label className="block text-xs text-on-surface/50 mb-1">Time frame</label>
                      <select
                        value={formData.time_frame}
                        onChange={(e) => setFormData({ ...formData, time_frame: e.target.value })}
                        className="w-full p-2 bg-surface-container-low border ghost-border rounded-lg text-on-surface/90 text-sm focus:border-purple-500/50 focus:outline-none"
                      >
                        <option value="3w">3 weeks</option>
                        <option value="8w">8 weeks</option>
                        <option value="12w">12 weeks</option>
                        <option value="16w">16 weeks</option>
                        <option value="6m">6 months</option>
                      </select>
                    </div>
                  </div>

                  <div>
                    <label className="block text-xs text-on-surface/50 mb-1">
                      Sport types <span className="text-on-surface/30">(comma-separated, empty = all)</span>
                    </label>
                    <input
                      type="text"
                      placeholder="Run, Ride, Swim"
                      value={formData.sport_types}
                      onChange={(e) => setFormData({ ...formData, sport_types: e.target.value })}
                      className="w-full p-2 bg-surface-container-low border ghost-border rounded-lg text-on-surface/90 text-sm placeholder-white/30 focus:border-purple-500/50 focus:outline-none"
                    />
                  </div>

                  <div className="flex items-center gap-4">
                    <label className="flex items-center gap-2 cursor-pointer">
                      <input
                        type="radio"
                        name="detail_mode"
                        checked={formData.detail_mode === 'summary'}
                        onChange={() => setFormData({ ...formData, detail_mode: 'summary' })}
                        className="text-purple-500 focus:ring-purple-500"
                      />
                      <span className="text-xs text-on-surface/60">Summary</span>
                    </label>
                    <label className="flex items-center gap-2 cursor-pointer">
                      <input
                        type="radio"
                        name="detail_mode"
                        checked={formData.detail_mode === 'detailed'}
                        onChange={() => setFormData({ ...formData, detail_mode: 'detailed' })}
                        className="text-purple-500 focus:ring-purple-500"
                      />
                      <span className="text-xs text-on-surface/60">Detailed (laps, splits)</span>
                    </label>
                  </div>

                  <label className="flex items-center gap-2 cursor-pointer">
                    <input
                      type="checkbox"
                      checked={formData.athlete_profile}
                      onChange={(e) => setFormData({ ...formData, athlete_profile: e.target.checked })}
                      className="w-3.5 h-3.5 rounded ghost-border text-purple-500 focus:ring-purple-500 bg-surface-container-low"
                    />
                    <span className="text-xs text-on-surface/60">Also fetch athlete profile</span>
                  </label>
                </div>
              )}
            </div>

            {/* Actions */}
            <div className="flex items-center gap-3 pt-4 border-t">
              <Button
                type="submit"
                disabled={createMutation.isPending || updateMutation.isPending}
              >
                {createMutation.isPending || updateMutation.isPending ? (
                  <span className="flex items-center gap-2">
                    <div className="pierre-spinner w-4 h-4"></div>
                    Saving...
                  </span>
                ) : (
                  isCreating ? 'Create Coach' : 'Save Changes'
                )}
              </Button>
              <Button
                type="button"
                variant="secondary"
                onClick={() => {
                  setIsCreating(false);
                  setIsEditing(false);
                  setFormData(defaultFormData);
                  setSelectedCoach(null);
                }}
              >
                Cancel
              </Button>
            </div>
          </form>
        </Card>
      </div>
    );
  }

  // Coach detail view - TypeScript guard for selectedCoach
  if (!selectedCoach) {
    return null;
  }

  return (
    <div className="max-w-3xl mx-auto">
      {/* Coach Details Card */}
      <Card variant="dark">
        {/* Card header with integrated back button - industry standard pattern */}
        <div className="flex items-center justify-between pb-4 mb-6 border-b ghost-border">
          <div className="flex items-center gap-3">
            <button
              onClick={() => setSelectedCoach(null)}
              className="p-1.5 rounded-lg text-outline hover:text-pierre-violet hover:bg-surface-container-low transition-colors"
              title="Back to coaches"
            >
              <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
              </svg>
            </button>
            <h2 className="text-2xl font-semibold text-on-surface">{selectedCoach.title}</h2>
            <span className={clsx(
              'px-2 py-1 text-xs font-medium rounded-full border',
              CATEGORY_COLORS[selectedCoach.category] || CATEGORY_COLORS.Custom
            )}>
              {selectedCoach.category}
            </span>
            <button
              onClick={(e) => handleToggleFavorite(e, selectedCoach.id)}
              className="text-outline hover:text-pierre-yellow-500 transition-colors"
              title={selectedCoach.is_favorite ? 'Remove from favorites' : 'Add to favorites'}
              aria-label={selectedCoach.is_favorite ? 'Remove from favorites' : 'Add to favorites'}
            >
              <svg
                className={clsx('w-6 h-6', selectedCoach.is_favorite ? 'fill-pierre-yellow-400 text-pierre-yellow-400' : 'fill-none')}
                stroke="currentColor"
                viewBox="0 0 24 24"
                aria-hidden="true"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M11.049 2.927c.3-.921 1.603-.921 1.902 0l1.519 4.674a1 1 0 00.95.69h4.915c.969 0 1.371 1.24.588 1.81l-3.976 2.888a1 1 0 00-.363 1.118l1.518 4.674c.3.922-.755 1.688-1.538 1.118l-3.976-2.888a1 1 0 00-1.176 0l-3.976 2.888c-.783.57-1.838-.197-1.538-1.118l1.518-4.674a1 1 0 00-.363-1.118l-3.976-2.888c-.784-.57-.38-1.81.588-1.81h4.914a1 1 0 00.951-.69l1.519-4.674z"
                />
              </svg>
            </button>
          </div>
          <div className="flex items-center gap-2">
            <Button
              variant="secondary"
              onClick={() => setIsEditing(true)}
            >
              <svg className="w-4 h-4 mr-2" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z" />
              </svg>
              Edit
            </Button>
            <Button
              variant="danger"
              onClick={handleDelete}
              disabled={deleteMutation.isPending}
            >
              {deleteMutation.isPending ? 'Deleting...' : 'Delete'}
            </Button>
          </div>
        </div>

        {/* Description */}
        {selectedCoach.description && (
          <p className="text-on-surface-variant mb-6">{selectedCoach.description}</p>
        )}

        {/* Stats */}
        <div className="grid grid-cols-3 gap-4 mb-6 p-4 bg-surface-container-low rounded-lg">
          <div className="text-center">
            <div className="text-2xl font-bold text-pierre-violet">
              ~{selectedCoach.token_count.toLocaleString()}
            </div>
            <div className="text-xs text-outline">
              Tokens ({getContextPercentage(selectedCoach.token_count)}% context)
            </div>
          </div>
          <div className="text-center">
            <div className="text-2xl font-bold text-pierre-activity">{selectedCoach.use_count}</div>
            <div className="text-xs text-outline">Uses</div>
          </div>
          <div className="text-center">
            <div className="text-2xl font-bold text-pierre-nutrition">
              {selectedCoach.is_favorite ? '★' : '☆'}
            </div>
            <div className="text-xs text-outline">
              {selectedCoach.is_favorite ? 'Favorite' : 'Not Favorite'}
            </div>
          </div>
        </div>

        {/* Structured Sections (when available) or flat System Prompt */}
        {selectedCoach.purpose || selectedCoach.instructions ? (
          <div className="space-y-4 mb-6">
            {selectedCoach.purpose && (
              <div>
                <h3 className="text-sm font-medium text-on-surface mb-2">Purpose</h3>
                <div className="p-4 bg-surface-container-low rounded-lg text-sm text-on-surface whitespace-pre-wrap">
                  {selectedCoach.purpose}
                </div>
              </div>
            )}
            {selectedCoach.when_to_use && (
              <div>
                <h3 className="text-sm font-medium text-on-surface mb-2">When to Use</h3>
                <div className="p-4 bg-surface-container-low rounded-lg text-sm text-on-surface whitespace-pre-wrap">
                  {selectedCoach.when_to_use}
                </div>
              </div>
            )}
            {selectedCoach.instructions && (
              <div>
                <h3 className="text-sm font-medium text-on-surface mb-2">Instructions</h3>
                <div className="p-4 bg-surface-container-low rounded-lg font-mono text-sm text-on-surface whitespace-pre-wrap max-h-48 overflow-y-auto">
                  {selectedCoach.instructions}
                </div>
              </div>
            )}
            {selectedCoach.example_inputs && (
              <div>
                <h3 className="text-sm font-medium text-on-surface mb-2">Example Inputs</h3>
                <div className="p-4 bg-surface-container-low rounded-lg text-sm text-on-surface whitespace-pre-wrap">
                  {selectedCoach.example_inputs}
                </div>
              </div>
            )}
            {selectedCoach.example_outputs && (
              <div>
                <h3 className="text-sm font-medium text-on-surface mb-2">Example Outputs</h3>
                <div className="p-4 bg-surface-container-low rounded-lg text-sm text-on-surface whitespace-pre-wrap">
                  {selectedCoach.example_outputs}
                </div>
              </div>
            )}
            {selectedCoach.success_criteria && (
              <div>
                <h3 className="text-sm font-medium text-on-surface mb-2">Success Criteria</h3>
                <div className="p-4 bg-surface-container-low rounded-lg text-sm text-on-surface whitespace-pre-wrap">
                  {selectedCoach.success_criteria}
                </div>
              </div>
            )}
          </div>
        ) : (
          <div className="mb-6">
            <h3 className="text-sm font-medium text-on-surface mb-2">System Prompt</h3>
            <div className="p-4 bg-surface-container-low rounded-lg font-mono text-sm text-on-surface whitespace-pre-wrap max-h-48 overflow-y-auto">
              {selectedCoach.system_prompt}
            </div>
          </div>
        )}

        {/* Tags */}
        {selectedCoach.tags.length > 0 && (
          <div className="mb-6">
            <h3 className="text-sm font-medium text-on-surface mb-2">Tags</h3>
            <div className="flex flex-wrap gap-2">
              {selectedCoach.tags.map((tag) => (
                <span key={tag} className="px-3 py-1 text-sm bg-surface-container-low text-on-surface rounded-full">
                  {tag}
                </span>
              ))}
            </div>
          </div>
        )}

        {/* Timestamps */}
        <div className="grid grid-cols-2 gap-4 text-sm text-outline pt-4 border-t ghost-border">
          <div>
            <span className="font-medium">Created:</span>{' '}
            {new Date(selectedCoach.created_at).toLocaleString()}
          </div>
          <div>
            <span className="font-medium">Last Updated:</span>{' '}
            {new Date(selectedCoach.updated_at).toLocaleString()}
          </div>
        </div>
      </Card>
    </div>
  );
}
