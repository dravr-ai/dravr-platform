# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

# ABOUTME: CocoaPods spec of the local dravr-health Expo module — HealthKit read access for the device health sync
# ABOUTME: Autolinked from frontend-mobile/modules by expo-modules-autolinking; links HealthKit, nothing else

Pod::Spec.new do |s|
  s.name           = 'DravrHealth'
  s.version        = '1.0.0'
  s.summary        = 'Reads HealthKit for the Dravr device health sync'
  s.description    = 'Read-only HealthKit access: anchored reads, sleep windows and day totals as raw samples'
  s.license        = 'MIT OR Apache-2.0'
  s.author         = 'dravr.ai'
  s.homepage       = 'https://dravr.ai'
  s.platforms      = { :ios => '16.4' }
  s.swift_version  = '5.9'
  s.source         = { git: '' }
  s.static_framework = true

  s.dependency 'ExpoModulesCore'
  s.frameworks = 'HealthKit'

  s.source_files = '**/*.{h,m,swift}'
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    'SWIFT_COMPILATION_MODE' => 'wholemodule'
  }
end
