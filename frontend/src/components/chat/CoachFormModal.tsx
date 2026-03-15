// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Modal for creating and editing custom coaches
// ABOUTME: Includes form fields for title, description, system prompt, and category

import type { CoachFormData } from './types';

interface CoachFormModalProps {
  isOpen: boolean;
  isEditing: boolean;
  formData: CoachFormData;
  onFormDataChange: (data: CoachFormData) => void;
  onSubmit: () => void;
  onClose: () => void;
  isSubmitting: boolean;
  submitError: boolean;
}

export default function CoachFormModal({
  isOpen,
  isEditing,
  formData,
  onFormDataChange,
  onSubmit,
  onClose,
  isSubmitting,
  submitError,
}: CoachFormModalProps) {
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
      <div className="relative bg-white rounded-2xl shadow-2xl max-w-lg w-full mx-4 max-h-[90vh] overflow-y-auto">
        <div className="p-6">
          {/* Close button */}
          <button
            onClick={onClose}
            className="absolute top-4 right-4 p-2 text-pierre-gray-400 hover:text-pierre-gray-600 hover:bg-pierre-gray-100 rounded-lg transition-colors"
            aria-label="Close"
          >
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>

          <div className="text-center mb-6">
            <div className="w-12 h-12 bg-pierre-violet/10 rounded-xl flex items-center justify-center mx-auto mb-4">
              <svg className="w-6 h-6 text-pierre-violet" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0z" />
              </svg>
            </div>
            <h2 className="text-xl font-semibold text-pierre-gray-900 mb-2">
              {isEditing ? 'Edit Coach' : 'Create Custom Coach'}
            </h2>
            <p className="text-pierre-gray-500 text-sm">
              {isEditing
                ? 'Update your coaching persona settings'
                : 'Define a specialized AI coaching persona for your training'}
            </p>
          </div>

          <form onSubmit={handleSubmit} className="space-y-4">
            <div>
              <label className="block text-sm font-medium text-pierre-gray-700 mb-1">
                Coach Name
              </label>
              <input
                type="text"
                placeholder="e.g., Marathon Training Coach"
                value={formData.title}
                onChange={(e) => onFormDataChange({ ...formData, title: e.target.value })}
                className="w-full px-3 py-2 text-sm border border-pierre-gray-200 rounded-lg focus:outline-none focus:ring-2 focus:ring-pierre-violet focus:border-transparent"
                required
              />
            </div>

            <div>
              <label className="block text-sm font-medium text-pierre-gray-700 mb-1">
                Description <span className="text-pierre-gray-400">(optional)</span>
              </label>
              <input
                type="text"
                placeholder="Brief description of what this coach specializes in"
                value={formData.description}
                onChange={(e) => onFormDataChange({ ...formData, description: e.target.value })}
                className="w-full px-3 py-2 text-sm border border-pierre-gray-200 rounded-lg focus:outline-none focus:ring-2 focus:ring-pierre-violet focus:border-transparent"
              />
            </div>

            <div>
              <label className="block text-sm font-medium text-pierre-gray-700 mb-1">
                System Prompt
              </label>
              <textarea
                placeholder="Define your coach's personality, expertise, and communication style..."
                value={formData.system_prompt}
                onChange={(e) => onFormDataChange({ ...formData, system_prompt: e.target.value })}
                rows={4}
                className="w-full px-3 py-2 text-sm border border-pierre-gray-200 rounded-lg focus:outline-none focus:ring-2 focus:ring-pierre-violet focus:border-transparent resize-none"
                required
              />
              {formData.system_prompt && (
                <p className="text-xs text-pierre-gray-500 mt-1">
                  ~{Math.ceil(formData.system_prompt.length / 4)} tokens ({((Math.ceil(formData.system_prompt.length / 4) / 128000) * 100).toFixed(1)}% of context)
                </p>
              )}
            </div>

            <div>
              <label className="block text-sm font-medium text-pierre-gray-700 mb-1">
                Category
              </label>
              <select
                value={formData.category}
                onChange={(e) => onFormDataChange({ ...formData, category: e.target.value })}
                className="w-full px-3 py-2 text-sm border border-pierre-gray-200 rounded-lg focus:outline-none focus:ring-2 focus:ring-pierre-violet focus:border-transparent bg-white"
              >
                <option value="Training">Training</option>
                <option value="Nutrition">Nutrition</option>
                <option value="Recovery">Recovery</option>
                <option value="Recipes">Recipes</option>
                <option value="Mobility">Mobility</option>
                <option value="Analysis">Analysis</option>
                <option value="Custom">Custom</option>
              </select>
            </div>

            {/* Data Context Section */}
            <div className="border-t border-pierre-gray-100 pt-4">
              <h3 className="text-sm font-medium text-pierre-gray-700 mb-3">Data Context</h3>

              <div className="mb-3">
                <label className="block text-sm font-medium text-pierre-gray-700 mb-1">
                  Startup Query <span className="text-pierre-gray-400">(optional)</span>
                </label>
                <textarea
                  placeholder="What should the coach analyze on first message? e.g., Analyze my training load and identify trends"
                  value={formData.startup_query}
                  onChange={(e) => onFormDataChange({ ...formData, startup_query: e.target.value })}
                  rows={2}
                  className="w-full px-3 py-2 text-sm border border-pierre-gray-200 rounded-lg focus:outline-none focus:ring-2 focus:ring-pierre-violet focus:border-transparent resize-none"
                />
              </div>

              <label className="flex items-center gap-2 mb-3 cursor-pointer">
                <input
                  type="checkbox"
                  checked={formData.prefetch_enabled}
                  onChange={(e) => onFormDataChange({ ...formData, prefetch_enabled: e.target.checked })}
                  className="w-4 h-4 rounded border-pierre-gray-300 text-pierre-violet focus:ring-pierre-violet"
                />
                <span className="text-sm text-pierre-gray-700">Pre-fetch activity data when conversation starts</span>
              </label>

              {formData.prefetch_enabled && (
                <div className="space-y-3 pl-6 border-l-2 border-pierre-violet/20">
                  <div className="grid grid-cols-2 gap-3">
                    <div>
                      <label className="block text-xs font-medium text-pierre-gray-600 mb-1">Activity count</label>
                      <input
                        type="number"
                        min={1}
                        max={200}
                        value={formData.activity_count}
                        onChange={(e) => onFormDataChange({ ...formData, activity_count: Math.max(1, Math.min(200, Number(e.target.value))) })}
                        className="w-full px-2 py-1.5 text-sm border border-pierre-gray-200 rounded-lg focus:outline-none focus:ring-2 focus:ring-pierre-violet focus:border-transparent"
                      />
                    </div>
                    <div>
                      <label className="block text-xs font-medium text-pierre-gray-600 mb-1">Time frame</label>
                      <select
                        value={formData.time_frame}
                        onChange={(e) => onFormDataChange({ ...formData, time_frame: e.target.value })}
                        className="w-full px-2 py-1.5 text-sm border border-pierre-gray-200 rounded-lg focus:outline-none focus:ring-2 focus:ring-pierre-violet focus:border-transparent bg-white"
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
                    <label className="block text-xs font-medium text-pierre-gray-600 mb-1">
                      Sport types <span className="text-pierre-gray-400">(none = all types)</span>
                    </label>
                    <div className="flex flex-wrap gap-1.5">
                      {['Run', 'Ride', 'Swim', 'Walk', 'Hike'].map((sport) => {
                        const isSelected = formData.sport_types.includes(sport);
                        return (
                          <button
                            key={sport}
                            type="button"
                            onClick={() => {
                              const updated = isSelected
                                ? formData.sport_types.filter((s) => s !== sport)
                                : [...formData.sport_types, sport];
                              onFormDataChange({ ...formData, sport_types: updated });
                            }}
                            className={`px-2.5 py-1 text-xs rounded-full border transition-colors ${
                              isSelected
                                ? 'bg-pierre-violet text-white border-pierre-violet'
                                : 'bg-white text-pierre-gray-600 border-pierre-gray-200 hover:border-pierre-violet/50'
                            }`}
                          >
                            {sport}
                          </button>
                        );
                      })}
                    </div>
                  </div>

                  <div className="flex items-center gap-4">
                    <label className="flex items-center gap-2 cursor-pointer">
                      <input
                        type="radio"
                        name="detail_mode"
                        checked={formData.detail_mode === 'summary'}
                        onChange={() => onFormDataChange({ ...formData, detail_mode: 'summary' })}
                        className="text-pierre-violet focus:ring-pierre-violet"
                      />
                      <span className="text-xs text-pierre-gray-600">Summary</span>
                    </label>
                    <label className="flex items-center gap-2 cursor-pointer">
                      <input
                        type="radio"
                        name="detail_mode"
                        checked={formData.detail_mode === 'detailed'}
                        onChange={() => onFormDataChange({ ...formData, detail_mode: 'detailed' })}
                        className="text-pierre-violet focus:ring-pierre-violet"
                      />
                      <span className="text-xs text-pierre-gray-600">Detailed (laps, splits)</span>
                    </label>
                  </div>

                  <label className="flex items-center gap-2 cursor-pointer">
                    <input
                      type="checkbox"
                      checked={formData.athlete_profile}
                      onChange={(e) => onFormDataChange({ ...formData, athlete_profile: e.target.checked })}
                      className="w-3.5 h-3.5 rounded border-pierre-gray-300 text-pierre-violet focus:ring-pierre-violet"
                    />
                    <span className="text-xs text-pierre-gray-600">Also fetch athlete profile</span>
                  </label>
                </div>
              )}
            </div>

            <div className="flex gap-3 pt-2">
              <button
                type="button"
                onClick={onClose}
                className="flex-1 px-4 py-2 text-sm font-medium text-pierre-gray-600 bg-pierre-gray-100 rounded-lg hover:bg-pierre-gray-200 transition-colors"
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={isSubmitting || !formData.title.trim() || !formData.system_prompt.trim()}
                className="flex-1 px-4 py-2 text-sm font-medium text-white bg-pierre-violet rounded-lg hover:bg-pierre-violet/90 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
              >
                {isEditing
                  ? (isSubmitting ? 'Saving...' : 'Save Changes')
                  : (isSubmitting ? 'Creating...' : 'Create Coach')}
              </button>
            </div>

            {submitError && (
              <p className="text-xs text-pierre-red-500 text-center">
                Failed to {isEditing ? 'update' : 'create'} coach. Please try again.
              </p>
            )}
          </form>
        </div>
      </div>
    </div>
  );
}
