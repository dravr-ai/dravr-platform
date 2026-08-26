// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Import a coach from a markdown file or a URL, with the server's preview before saving
// ABOUTME: Lives in the "Your coaches" header on Discover; the imported coach lands in that grid

import { useCallback, useEffect, useRef, useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { clsx } from 'clsx';
import { FileText, Link2, Upload } from 'lucide-react';
import type { ImportPreviewResponse } from '@pierre/shared-types';
import { coachesApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { Button, Input, Modal, ModalActions } from '../ui';
import { categoryBadgeClass } from './coachCategory';

type PendingSource = { type: 'file'; content: string } | { type: 'url'; url: string };

/**
 * The import affordance: a button with a two-item menu, a URL dialog, and the
 * preview dialog the server's validation feeds. Saving invalidates the coach
 * list so the new coach appears in the grid this sits above.
 */
export default function CoachImport() {
  const queryClient = useQueryClient();
  const [menuOpen, setMenuOpen] = useState(false);
  const [urlDialogOpen, setUrlDialogOpen] = useState(false);
  const [importUrl, setImportUrl] = useState('');
  const [preview, setPreview] = useState<ImportPreviewResponse | null>(null);
  const [pendingSource, setPendingSource] = useState<PendingSource | null>(null);
  const [importError, setImportError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const invalidateCoaches = () => queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });

  const previewMarkdown = useMutation({
    mutationFn: (markdown: string) => coachesApi.importPreview(markdown),
    onSuccess: (data) => {
      setPreview(data);
      setImportError(null);
    },
    onError: (error: Error) => {
      setImportError(error.message || 'Failed to preview import');
      setPreview(null);
    },
  });

  const saveMarkdown = useMutation({
    mutationFn: (markdown: string) => coachesApi.importFromMarkdown(markdown),
    onSuccess: () => {
      invalidateCoaches();
      setPreview(null);
      setPendingSource(null);
      setImportError(null);
    },
    onError: (error: Error) => setImportError(error.message || 'Failed to import coach'),
  });

  const previewUrl = useMutation({
    mutationFn: (url: string) => coachesApi.importFromUrl(url, false),
    onSuccess: (data) => {
      setPreview(data as ImportPreviewResponse);
      setUrlDialogOpen(false);
      setImportError(null);
    },
    onError: (error: Error) => setImportError(error.message || 'Failed to fetch URL'),
  });

  const saveUrl = useMutation({
    mutationFn: (url: string) => coachesApi.importFromUrl(url, true),
    onSuccess: () => {
      invalidateCoaches();
      setPreview(null);
      setPendingSource(null);
      setImportError(null);
    },
    onError: (error: Error) => setImportError(error.message || 'Failed to import coach from URL'),
  });

  useEffect(() => {
    if (!menuOpen) return;
    const onPointerDown = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener('mousedown', onPointerDown);
    return () => document.removeEventListener('mousedown', onPointerDown);
  }, [menuOpen]);

  const handleFile = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;
      const reader = new FileReader();
      reader.onload = (event) => {
        const content = event.target?.result as string;
        setPendingSource({ type: 'file', content });
        previewMarkdown.mutate(content);
      };
      reader.readAsText(file);
      // Reset so the same file can be picked again.
      e.target.value = '';
    },
    [previewMarkdown],
  );

  const submitUrl = () => {
    const trimmed = importUrl.trim();
    if (!trimmed) return;
    setPendingSource({ type: 'url', url: trimmed });
    previewUrl.mutate(trimmed);
  };

  const confirmImport = () => {
    if (!pendingSource) return;
    if (pendingSource.type === 'file') saveMarkdown.mutate(pendingSource.content);
    else saveUrl.mutate(pendingSource.url);
  };

  const cancelImport = () => {
    setPreview(null);
    setPendingSource(null);
    setImportError(null);
  };

  const isSaving = saveMarkdown.isPending || saveUrl.isPending;

  return (
    <>
      <div className="relative" ref={menuRef}>
        <button
          type="button"
          onClick={() => setMenuOpen((v) => !v)}
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          className="p-2 rounded-lg text-on-surface-variant hover:text-primary hover:bg-surface-container-low transition-colors min-w-[44px] min-h-[44px] flex items-center justify-center"
          title="Import Coach"
          aria-label="Import Coach"
        >
          <Upload className="w-4 h-4" aria-hidden="true" />
        </button>
        {menuOpen && (
          <div
            role="menu"
            aria-label="Import a coach"
            className="absolute right-0 top-full mt-1 w-48 bg-surface rounded-lg border ghost-border shadow-xl z-30 py-1"
          >
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                setMenuOpen(false);
                fileInputRef.current?.click();
              }}
              className="w-full flex items-center gap-2 px-3 py-2 text-sm text-on-surface hover:bg-surface-container-low transition-colors min-h-[44px]"
            >
              <FileText className="w-4 h-4" aria-hidden="true" />
              Import from File
            </button>
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                setMenuOpen(false);
                setImportUrl('');
                setImportError(null);
                setUrlDialogOpen(true);
              }}
              className="w-full flex items-center gap-2 px-3 py-2 text-sm text-on-surface hover:bg-surface-container-low transition-colors min-h-[44px]"
            >
              <Link2 className="w-4 h-4" aria-hidden="true" />
              Import from URL
            </button>
          </div>
        )}
      </div>
      {/* File pickers have no editorial primitive; the control is hidden and
          driven by the menu item above. */}
      <input
        ref={fileInputRef}
        type="file"
        accept=".md,text/markdown"
        className="hidden"
        onChange={handleFile}
        aria-hidden="true"
        tabIndex={-1}
      />

      <Modal
        isOpen={urlDialogOpen}
        onClose={() => {
          setUrlDialogOpen(false);
          setImportError(null);
        }}
        title="Import from URL"
        size="md"
        footer={
          <ModalActions>
            <Button
              variant="secondary"
              onClick={() => {
                setUrlDialogOpen(false);
                setImportError(null);
              }}
            >
              Cancel
            </Button>
            <Button onClick={submitUrl} disabled={!importUrl.trim() || previewUrl.isPending} loading={previewUrl.isPending}>
              {previewUrl.isPending ? 'Fetching...' : 'Preview'}
            </Button>
          </ModalActions>
        }
      >
        <p className="text-sm text-on-surface-variant mb-4">Enter the URL of a markdown coach file.</p>
        <Input
          type="url"
          label="Coach file URL"
          value={importUrl}
          onChange={(e) => setImportUrl(e.target.value)}
          placeholder="https://example.com/coach.md"
          autoFocus
          onKeyDown={(e) => {
            if (e.key === 'Enter') submitUrl();
          }}
        />
        {importError && <p className="text-sm text-error mt-3">{importError}</p>}
      </Modal>

      <Modal
        isOpen={preview !== null}
        onClose={cancelImport}
        title="Import Preview"
        size="lg"
        footer={
          preview?.valid ? (
            <ModalActions>
              <Button variant="secondary" onClick={cancelImport}>
                Cancel
              </Button>
              <Button onClick={confirmImport} disabled={isSaving} loading={isSaving}>
                {isSaving ? 'Importing...' : 'Import'}
              </Button>
            </ModalActions>
          ) : (
            <ModalActions>
              <Button variant="secondary" onClick={cancelImport}>
                Close
              </Button>
            </ModalActions>
          )
        }
      >
        {preview && !preview.valid && (
          <div>
            <p className="text-sm text-error mb-3">This file cannot be imported:</p>
            {preview.errors && preview.errors.length > 0 && (
              <ul className="list-disc list-inside text-sm text-error space-y-1">
                {preview.errors.map((err, i) => (
                  <li key={i}>{err}</li>
                ))}
              </ul>
            )}
          </div>
        )}
        {preview?.valid && (
          <div className="space-y-4">
            {preview.parsed && (
              <div className="space-y-2">
                <div className="flex items-center gap-2">
                  <span className="text-sm text-on-surface-variant">Name:</span>
                  <span className="text-sm text-on-surface font-medium">{preview.parsed.title}</span>
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-sm text-on-surface-variant">Category:</span>
                  <span
                    className={clsx(
                      'px-2 py-0.5 text-xs font-medium rounded-full border capitalize',
                      categoryBadgeClass(preview.parsed.category),
                    )}
                  >
                    {preview.parsed.category}
                  </span>
                </div>
                {preview.parsed.tags.length > 0 && (
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="text-sm text-on-surface-variant">Tags:</span>
                    {preview.parsed.tags.map((tag) => (
                      <span key={tag} className="px-2 py-0.5 text-xs bg-surface-container-low text-on-surface rounded-full">
                        {tag}
                      </span>
                    ))}
                  </div>
                )}
                {preview.token_count !== undefined && (
                  <div className="flex items-center gap-2">
                    <span className="text-sm text-on-surface-variant">Tokens:</span>
                    <span className="text-sm text-on-surface">~{preview.token_count.toLocaleString()}</span>
                  </div>
                )}
                <div className="flex items-center gap-3 text-xs text-on-surface-variant">
                  <span>Purpose: {preview.parsed.purpose ? 'Yes' : 'No'}</span>
                  <span>Instructions: {preview.parsed.has_instructions ? 'Yes' : 'No'}</span>
                  <span>Examples: {preview.parsed.has_example_inputs ? 'Yes' : 'No'}</span>
                </div>
              </div>
            )}
            {preview.duplicate_exists && (
              <div className="p-3 rounded-lg bg-warning/10 border border-warning/20">
                <p className="text-sm text-warning">
                  A coach with matching content already exists. Importing will create a duplicate.
                </p>
              </div>
            )}
            {preview.warnings && preview.warnings.length > 0 && (
              <div className="p-3 rounded-lg bg-warning/10 border border-warning/20">
                <p className="text-sm font-medium text-warning mb-1">Warnings:</p>
                <ul className="list-disc list-inside text-sm text-warning space-y-1">
                  {preview.warnings.map((warn, i) => (
                    <li key={i}>{warn}</li>
                  ))}
                </ul>
              </div>
            )}
            {importError && <p className="text-sm text-error">{importError}</p>}
          </div>
        )}
      </Modal>
    </>
  );
}
