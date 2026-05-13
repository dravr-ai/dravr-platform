// ABOUTME: Per-category parameter definition builders for AdminConfigService
// ABOUTME: Extracted from service::initialize_definitions to keep that bootstrap fn navigable

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use crate::config::admin::service::ParameterDefinition;
use crate::config::admin::types::{ConfigDataType, ParameterRange};

/// Local helper for inserting a [`ParameterDefinition`] keyed on its own `key`.
///
/// A mirror of the same-named associated fn on `AdminConfigService`; lives
/// here so per-category builders can run without going through the service.
fn add_definition(defs: &mut HashMap<String, ParameterDefinition>, def: ParameterDefinition) {
    defs.insert(def.key.clone(), def);
}

/// Register the `llm_pricing.*` catalog entries — one input + output price
/// per (provider, model) pair surfaced to the admin pricing dashboard.
pub(super) fn register_llm_pricing(defs: &mut HashMap<String, ParameterDefinition>) {
    // Google Gemini 2.0 Flash
    add_definition(
        defs,
        ParameterDefinition {
            key: "llm_pricing.gemini.gemini-2.0-flash.input_per_million".to_owned(),
            display_name: "Gemini 2.0 Flash Input Price".to_owned(),
            description: "USD per million input tokens for Google Gemini 2.0 Flash".to_owned(),
            category: "llm_pricing".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(0.075),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(0.0),
                max: serde_json::json!(100.0),
                step: Some(0.001),
            }),
            enum_options: None,
            units: Some("$/M tokens".to_owned()),
            scientific_basis: None,
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
    add_definition(
        defs,
        ParameterDefinition {
            key: "llm_pricing.gemini.gemini-2.0-flash.output_per_million".to_owned(),
            display_name: "Gemini 2.0 Flash Output Price".to_owned(),
            description: "USD per million output tokens for Google Gemini 2.0 Flash".to_owned(),
            category: "llm_pricing".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(0.30),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(0.0),
                max: serde_json::json!(100.0),
                step: Some(0.001),
            }),
            enum_options: None,
            units: Some("$/M tokens".to_owned()),
            scientific_basis: None,
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    // Google Gemini 2.5 Pro
    add_definition(
        defs,
        ParameterDefinition {
            key: "llm_pricing.gemini.gemini-2.5-pro.input_per_million".to_owned(),
            display_name: "Gemini 2.5 Pro Input Price".to_owned(),
            description: "USD per million input tokens for Google Gemini 2.5 Pro".to_owned(),
            category: "llm_pricing".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(1.25),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(0.0),
                max: serde_json::json!(100.0),
                step: Some(0.01),
            }),
            enum_options: None,
            units: Some("$/M tokens".to_owned()),
            scientific_basis: None,
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
    add_definition(
        defs,
        ParameterDefinition {
            key: "llm_pricing.gemini.gemini-2.5-pro.output_per_million".to_owned(),
            display_name: "Gemini 2.5 Pro Output Price".to_owned(),
            description: "USD per million output tokens for Google Gemini 2.5 Pro".to_owned(),
            category: "llm_pricing".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(10.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(0.0),
                max: serde_json::json!(100.0),
                step: Some(0.01),
            }),
            enum_options: None,
            units: Some("$/M tokens".to_owned()),
            scientific_basis: None,
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    // Google Gemini 2.5 Flash
    add_definition(
        defs,
        ParameterDefinition {
            key: "llm_pricing.gemini.gemini-2.5-flash.input_per_million".to_owned(),
            display_name: "Gemini 2.5 Flash Input Price".to_owned(),
            description: "USD per million input tokens for Google Gemini 2.5 Flash".to_owned(),
            category: "llm_pricing".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(0.15),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(0.0),
                max: serde_json::json!(100.0),
                step: Some(0.001),
            }),
            enum_options: None,
            units: Some("$/M tokens".to_owned()),
            scientific_basis: None,
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
    add_definition(
        defs,
        ParameterDefinition {
            key: "llm_pricing.gemini.gemini-2.5-flash.output_per_million".to_owned(),
            display_name: "Gemini 2.5 Flash Output Price".to_owned(),
            description: "USD per million output tokens for Google Gemini 2.5 Flash".to_owned(),
            category: "llm_pricing".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(0.60),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(0.0),
                max: serde_json::json!(100.0),
                step: Some(0.001),
            }),
            enum_options: None,
            units: Some("$/M tokens".to_owned()),
            scientific_basis: None,
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    // Groq LLaMA 3.3 70B
    add_definition(
        defs,
        ParameterDefinition {
            key: "llm_pricing.groq.llama-3.3-70b.input_per_million".to_owned(),
            display_name: "LLaMA 3.3 70B Input Price".to_owned(),
            description: "USD per million input tokens for Groq LLaMA 3.3 70B".to_owned(),
            category: "llm_pricing".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(0.59),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(0.0),
                max: serde_json::json!(100.0),
                step: Some(0.01),
            }),
            enum_options: None,
            units: Some("$/M tokens".to_owned()),
            scientific_basis: None,
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
    add_definition(
        defs,
        ParameterDefinition {
            key: "llm_pricing.groq.llama-3.3-70b.output_per_million".to_owned(),
            display_name: "LLaMA 3.3 70B Output Price".to_owned(),
            description: "USD per million output tokens for Groq LLaMA 3.3 70B".to_owned(),
            category: "llm_pricing".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(0.79),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(0.0),
                max: serde_json::json!(100.0),
                step: Some(0.01),
            }),
            enum_options: None,
            units: Some("$/M tokens".to_owned()),
            scientific_basis: None,
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    // Groq Mixtral
    add_definition(
        defs,
        ParameterDefinition {
            key: "llm_pricing.groq.mixtral.input_per_million".to_owned(),
            display_name: "Mixtral Input Price".to_owned(),
            description: "USD per million input tokens for Groq Mixtral".to_owned(),
            category: "llm_pricing".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(0.24),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(0.0),
                max: serde_json::json!(100.0),
                step: Some(0.01),
            }),
            enum_options: None,
            units: Some("$/M tokens".to_owned()),
            scientific_basis: None,
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
    add_definition(
        defs,
        ParameterDefinition {
            key: "llm_pricing.groq.mixtral.output_per_million".to_owned(),
            display_name: "Mixtral Output Price".to_owned(),
            description: "USD per million output tokens for Groq Mixtral".to_owned(),
            category: "llm_pricing".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(0.24),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(0.0),
                max: serde_json::json!(100.0),
                step: Some(0.01),
            }),
            enum_options: None,
            units: Some("$/M tokens".to_owned()),
            scientific_basis: None,
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    // Groq LLaMA 3.1 8B
    add_definition(
        defs,
        ParameterDefinition {
            key: "llm_pricing.groq.llama-3.1-8b.input_per_million".to_owned(),
            display_name: "LLaMA 3.1 8B Input Price".to_owned(),
            description: "USD per million input tokens for Groq LLaMA 3.1 8B".to_owned(),
            category: "llm_pricing".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(0.05),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(0.0),
                max: serde_json::json!(100.0),
                step: Some(0.001),
            }),
            enum_options: None,
            units: Some("$/M tokens".to_owned()),
            scientific_basis: None,
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
    add_definition(
        defs,
        ParameterDefinition {
            key: "llm_pricing.groq.llama-3.1-8b.output_per_million".to_owned(),
            display_name: "LLaMA 3.1 8B Output Price".to_owned(),
            description: "USD per million output tokens for Groq LLaMA 3.1 8B".to_owned(),
            category: "llm_pricing".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(0.08),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(0.0),
                max: serde_json::json!(100.0),
                step: Some(0.001),
            }),
            enum_options: None,
            units: Some("$/M tokens".to_owned()),
            scientific_basis: None,
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}

/// Register the `Rate Limiting Parameters` catalog entries.
pub(super) fn register_rate_limiting(defs: &mut HashMap<String, ParameterDefinition>) {
    // Rate Limiting Parameters
    add_definition(
        defs,
        ParameterDefinition {
            key: "rate_limit.free_tier_burst".to_owned(),
            display_name: "Free Tier Burst Limit".to_owned(),
            description: "Maximum burst requests for free tier users".to_owned(),
            category: "rate_limiting".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(10),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(1),
                max: serde_json::json!(100),
                step: Some(1.0),
            }),
            enum_options: None,
            units: Some("requests".to_owned()),
            scientific_basis: None,
            env_variable: Some("RATE_LIMIT_FREE_TIER_BURST".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "rate_limit.professional_burst".to_owned(),
            display_name: "Professional Tier Burst Limit".to_owned(),
            description: "Maximum burst requests for professional tier users".to_owned(),
            category: "rate_limiting".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(50),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(10),
                max: serde_json::json!(500),
                step: Some(1.0),
            }),
            enum_options: None,
            units: Some("requests".to_owned()),
            scientific_basis: None,
            env_variable: Some("RATE_LIMIT_PROFESSIONAL_BURST".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "rate_limit.enterprise_burst".to_owned(),
            display_name: "Enterprise Tier Burst Limit".to_owned(),
            description: "Maximum burst requests for enterprise tier users".to_owned(),
            category: "rate_limiting".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(100),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(50),
                max: serde_json::json!(10000),
                step: Some(1.0),
            }),
            enum_options: None,
            units: Some("requests".to_owned()),
            scientific_basis: None,
            env_variable: Some("RATE_LIMIT_ENTERPRISE_BURST".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}

/// Register the `Feature Flags` catalog entries.
pub(super) fn register_feature_flags(defs: &mut HashMap<String, ParameterDefinition>) {
    // Feature Flags
    add_definition(
        defs,
        ParameterDefinition {
            key: "feature.auto_approval_enabled".to_owned(),
            display_name: "Auto-Approve New Users".to_owned(),
            description: "Automatically approve new user registrations without admin intervention"
                .to_owned(),
            category: "feature_flags".to_owned(),
            data_type: ConfigDataType::Boolean,
            default_value: serde_json::json!(false),
            valid_range: None,
            enum_options: None,
            units: None,
            scientific_basis: None,
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "feature.weather_api_enabled".to_owned(),
            display_name: "Weather API Integration".to_owned(),
            description: "Enable weather data integration for activity analysis".to_owned(),
            category: "feature_flags".to_owned(),
            data_type: ConfigDataType::Boolean,
            default_value: serde_json::json!(true),
            valid_range: None,
            enum_options: None,
            units: None,
            scientific_basis: None,
            env_variable: Some("WEATHER_SERVICE_ENABLED".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}

/// Register the `LLM Provider Configuration` catalog entries.
pub(super) fn register_llm_provider_config(defs: &mut HashMap<String, ParameterDefinition>) {
    // LLM Provider Configuration
    add_definition(defs,
        ParameterDefinition {
            key: "llm.provider".to_owned(),
            display_name: "LLM Provider".to_owned(),
            description:
                "AI model provider for chat and recipe generation. Groq for fast cloud inference, Gemini for full features, Local for privacy-first self-hosted models"
                    .to_owned(),
            category: "llm_provider".to_owned(),
            data_type: ConfigDataType::Enum,
            default_value: serde_json::json!("groq"),
            valid_range: None,
            enum_options: Some(vec![
                "groq".to_owned(),
                "gemini".to_owned(),
                "local".to_owned(),
            ]),
            units: None,
            scientific_basis: None,
            env_variable: Some("PIERRE_LLM_PROVIDER".to_owned()),
            is_runtime_configurable: false,
            requires_restart: true,
        },
    );

    add_definition(defs,
        ParameterDefinition {
            key: "llm.local_base_url".to_owned(),
            display_name: "Local LLM Base URL".to_owned(),
            description:
                "Base URL for local LLM server (Ollama, vLLM, LocalAI). Only used when provider is 'local'"
                    .to_owned(),
            category: "llm_provider".to_owned(),
            data_type: ConfigDataType::String,
            default_value: serde_json::json!("http://localhost:11434/v1"),
            valid_range: None,
            enum_options: None,
            units: None,
            scientific_basis: None,
            env_variable: Some("LOCAL_LLM_BASE_URL".to_owned()),
            is_runtime_configurable: false,
            requires_restart: true,
        },
    );

    add_definition(defs,
        ParameterDefinition {
            key: "llm.local_model".to_owned(),
            display_name: "Local LLM Model".to_owned(),
            description:
                "Model name to use with local LLM server (e.g., qwen2.5:14b-instruct, llama3.1:8b-instruct)"
                    .to_owned(),
            category: "llm_provider".to_owned(),
            data_type: ConfigDataType::String,
            default_value: serde_json::json!("qwen2.5:14b-instruct"),
            valid_range: None,
            enum_options: None,
            units: None,
            scientific_basis: None,
            env_variable: Some("LOCAL_LLM_MODEL".to_owned()),
            is_runtime_configurable: false,
            requires_restart: true,
        },
    );
}

/// Register the `Heart Rate Zones` catalog entries.
pub(super) fn register_heart_rate_zones(defs: &mut HashMap<String, ParameterDefinition>) {
    // Heart Rate Zones
    add_definition(
        defs,
        ParameterDefinition {
            key: "heart_rate.recovery_zone".to_owned(),
            display_name: "Recovery Zone Max".to_owned(),
            description: "Maximum heart rate percentage for recovery zone (Zone 1)".to_owned(),
            category: "heart_rate_zones".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(60.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(50.0),
                max: serde_json::json!(70.0),
                step: Some(0.5),
            }),
            enum_options: None,
            units: Some("% max HR".to_owned()),
            scientific_basis: Some("Polarized Training Model".to_owned()),
            env_variable: Some("FITNESS_ZONE_RECOVERY_MAX".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "heart_rate.endurance_zone".to_owned(),
            display_name: "Endurance Zone Max".to_owned(),
            description: "Maximum heart rate percentage for endurance zone (Zone 2)".to_owned(),
            category: "heart_rate_zones".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(70.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(60.0),
                max: serde_json::json!(80.0),
                step: Some(0.5),
            }),
            enum_options: None,
            units: Some("% max HR".to_owned()),
            scientific_basis: Some("Maffetone Method".to_owned()),
            env_variable: Some("FITNESS_ZONE_ENDURANCE_MAX".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "heart_rate.tempo_zone".to_owned(),
            display_name: "Tempo Zone Max".to_owned(),
            description: "Maximum heart rate percentage for tempo zone (Zone 3)".to_owned(),
            category: "heart_rate_zones".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(80.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(70.0),
                max: serde_json::json!(90.0),
                step: Some(0.5),
            }),
            enum_options: None,
            units: Some("% max HR".to_owned()),
            scientific_basis: Some("Coggan & Allen 2006".to_owned()),
            env_variable: Some("FITNESS_ZONE_TEMPO_MAX".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "heart_rate.threshold_zone".to_owned(),
            display_name: "Threshold Zone Max".to_owned(),
            description: "Maximum heart rate percentage for threshold zone (Zone 4)".to_owned(),
            category: "heart_rate_zones".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(90.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(80.0),
                max: serde_json::json!(95.0),
                step: Some(0.5),
            }),
            enum_options: None,
            units: Some("% max HR".to_owned()),
            scientific_basis: Some("Seiler 2010".to_owned()),
            env_variable: Some("FITNESS_ZONE_THRESHOLD_MAX".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}

/// Register the `Algorithm Selection` catalog entries.
pub(super) fn register_algorithm_selection(defs: &mut HashMap<String, ParameterDefinition>) {
    // Algorithm Selection
    add_definition(
        defs,
        ParameterDefinition {
            key: "algorithm.tss".to_owned(),
            display_name: "TSS Calculation Method".to_owned(),
            description: "Algorithm for Training Stress Score calculation".to_owned(),
            category: "algorithms".to_owned(),
            data_type: ConfigDataType::Enum,
            default_value: serde_json::json!("avg_power"),
            valid_range: None,
            enum_options: Some(vec![
                "avg_power".to_owned(),
                "normalized_power".to_owned(),
                "hybrid".to_owned(),
            ]),
            units: None,
            scientific_basis: Some("Coggan's TSS methodology".to_owned()),
            env_variable: Some("PIERRE_TSS_ALGORITHM".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "algorithm.maxhr".to_owned(),
            display_name: "Max HR Estimation".to_owned(),
            description: "Algorithm for maximum heart rate estimation".to_owned(),
            category: "algorithms".to_owned(),
            data_type: ConfigDataType::Enum,
            default_value: serde_json::json!("tanaka"),
            valid_range: None,
            enum_options: Some(vec![
                "fox".to_owned(),
                "tanaka".to_owned(),
                "nes".to_owned(),
                "gulati".to_owned(),
            ]),
            units: None,
            scientific_basis: Some("Tanaka et al. 2001: 208 - 0.7 × age".to_owned()),
            env_variable: Some("PIERRE_MAXHR_ALGORITHM".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}

/// Register the `Recommendation Engine` catalog entries.
pub(super) fn register_recommendation_engine(defs: &mut HashMap<String, ParameterDefinition>) {
    // Recommendation Engine
    add_definition(
        defs,
        ParameterDefinition {
            key: "recommendation.low_weekly_distance_km".to_owned(),
            display_name: "Low Weekly Distance".to_owned(),
            description: "Distance threshold below which a low volume warning is triggered"
                .to_owned(),
            category: "recommendation_engine".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(20.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(5.0),
                max: serde_json::json!(50.0),
                step: Some(1.0),
            }),
            enum_options: None,
            units: Some("km".to_owned()),
            scientific_basis: None,
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "recommendation.high_weekly_distance_km".to_owned(),
            display_name: "High Weekly Distance".to_owned(),
            description: "Distance threshold above which overtraining warnings are triggered"
                .to_owned(),
            category: "recommendation_engine".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(80.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(30.0),
                max: serde_json::json!(200.0),
                step: Some(5.0),
            }),
            enum_options: None,
            units: Some("km".to_owned()),
            scientific_basis: None,
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "recommendation.max_per_category".to_owned(),
            display_name: "Max Recommendations Per Category".to_owned(),
            description: "Maximum number of recommendations to show per category".to_owned(),
            category: "recommendation_engine".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(3),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(1),
                max: serde_json::json!(10),
                step: Some(1.0),
            }),
            enum_options: None,
            units: None,
            scientific_basis: None,
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}

/// Register the `Sleep & Recovery` catalog entries.
pub(super) fn register_sleep_recovery(defs: &mut HashMap<String, ParameterDefinition>) {
    // Sleep & Recovery
    add_definition(
        defs,
        ParameterDefinition {
            key: "sleep.adult_min_hours".to_owned(),
            display_name: "Minimum Sleep Hours".to_owned(),
            description: "Minimum recommended sleep hours for adults".to_owned(),
            category: "sleep_recovery".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(7.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(5.0),
                max: serde_json::json!(8.0),
                step: Some(0.5),
            }),
            enum_options: None,
            units: Some("hours".to_owned()),
            scientific_basis: Some("National Sleep Foundation Guidelines".to_owned()),
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "sleep.deep_sleep_min_percent".to_owned(),
            display_name: "Minimum Deep Sleep".to_owned(),
            description: "Minimum percentage of deep sleep for quality rest".to_owned(),
            category: "sleep_recovery".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(15.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(10.0),
                max: serde_json::json!(20.0),
                step: Some(1.0),
            }),
            enum_options: None,
            units: Some("%".to_owned()),
            scientific_basis: Some("AASM Sleep Guidelines".to_owned()),
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}

/// Register the `Training Stress Balance` catalog entries.
pub(super) fn register_training_stress_balance(defs: &mut HashMap<String, ParameterDefinition>) {
    // Training Stress Balance
    add_definition(
        defs,
        ParameterDefinition {
            key: "tsb.fatigued_threshold".to_owned(),
            display_name: "Fatigued TSB Threshold".to_owned(),
            description: "TSB value below which athlete is considered fatigued".to_owned(),
            category: "training_stress".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(-10.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(-30.0),
                max: serde_json::json!(-5.0),
                step: Some(1.0),
            }),
            enum_options: None,
            units: Some("TSB".to_owned()),
            scientific_basis: Some("Banister's Impulse-Response Model".to_owned()),
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "tsb.fresh_min".to_owned(),
            display_name: "Fresh Range Minimum".to_owned(),
            description: "Minimum TSB value for optimal performance readiness".to_owned(),
            category: "training_stress".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(5.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(0.0),
                max: serde_json::json!(15.0),
                step: Some(1.0),
            }),
            enum_options: None,
            units: Some("TSB".to_owned()),
            scientific_basis: Some("Banister's Impulse-Response Model".to_owned()),
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}

/// Register the `Weather Analysis` catalog entries.
pub(super) fn register_weather_analysis(defs: &mut HashMap<String, ParameterDefinition>) {
    // Weather Analysis
    add_definition(
        defs,
        ParameterDefinition {
            key: "weather.ideal_min_celsius".to_owned(),
            display_name: "Ideal Min Temperature".to_owned(),
            description: "Minimum temperature for ideal training conditions".to_owned(),
            category: "weather_analysis".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(10.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(0.0),
                max: serde_json::json!(15.0),
                step: Some(1.0),
            }),
            enum_options: None,
            units: Some("°C".to_owned()),
            scientific_basis: None,
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "weather.ideal_max_celsius".to_owned(),
            display_name: "Ideal Max Temperature".to_owned(),
            description: "Maximum temperature for ideal training conditions".to_owned(),
            category: "weather_analysis".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(20.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(15.0),
                max: serde_json::json!(30.0),
                step: Some(1.0),
            }),
            enum_options: None,
            units: Some("°C".to_owned()),
            scientific_basis: None,
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}

/// Register the `Nutrition` catalog entries.
pub(super) fn register_nutrition(defs: &mut HashMap<String, ParameterDefinition>) {
    // Nutrition
    add_definition(
        defs,
        ParameterDefinition {
            key: "nutrition.protein_athlete_g_per_kg".to_owned(),
            display_name: "Athlete Protein Target".to_owned(),
            description: "Recommended protein intake for athletes per kg body weight".to_owned(),
            category: "nutrition".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(1.8),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(1.4),
                max: serde_json::json!(2.5),
                step: Some(0.1),
            }),
            enum_options: None,
            units: Some("g/kg".to_owned()),
            scientific_basis: Some("Phillips 2011, ISSN Position Stand".to_owned()),
            env_variable: None,
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}

/// Register the `Tokio Runtime Configuration` catalog entries.
pub(super) fn register_tokio_runtime(defs: &mut HashMap<String, ParameterDefinition>) {
    // Tokio Runtime Configuration
    add_definition(
        defs,
        ParameterDefinition {
            key: "tokio_runtime.worker_threads".to_owned(),
            display_name: "Worker Threads".to_owned(),
            description: "Number of Tokio runtime worker threads. Default: CPU core count"
                .to_owned(),
            category: "tokio_runtime".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(null),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(1),
                max: serde_json::json!(256),
                step: Some(1.0),
            }),
            enum_options: None,
            units: Some("threads".to_owned()),
            scientific_basis: None,
            env_variable: Some("TOKIO_WORKER_THREADS".to_owned()),
            is_runtime_configurable: false,
            requires_restart: true,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "tokio_runtime.thread_stack_size".to_owned(),
            display_name: "Thread Stack Size".to_owned(),
            description: "Stack size for worker threads in bytes. Default: ~2MB".to_owned(),
            category: "tokio_runtime".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(null),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(524_288),
                max: serde_json::json!(16_777_216),
                step: Some(524_288.0),
            }),
            enum_options: None,
            units: Some("bytes".to_owned()),
            scientific_basis: None,
            env_variable: Some("TOKIO_THREAD_STACK_SIZE".to_owned()),
            is_runtime_configurable: false,
            requires_restart: true,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "tokio_runtime.thread_name".to_owned(),
            display_name: "Thread Name Prefix".to_owned(),
            description: "Name prefix for worker threads".to_owned(),
            category: "tokio_runtime".to_owned(),
            data_type: ConfigDataType::String,
            default_value: serde_json::json!("pierre-worker"),
            valid_range: None,
            enum_options: None,
            units: None,
            scientific_basis: None,
            env_variable: Some("TOKIO_THREAD_NAME".to_owned()),
            is_runtime_configurable: false,
            requires_restart: true,
        },
    );
}

/// Register the `SQLx Connection Pool Configuration` catalog entries.
pub(super) fn register_sqlx_pool(defs: &mut HashMap<String, ParameterDefinition>) {
    // SQLx Connection Pool Configuration
    add_definition(
        defs,
        ParameterDefinition {
            key: "sqlx.idle_timeout_secs".to_owned(),
            display_name: "Idle Timeout".to_owned(),
            description:
                "Maximum time a connection can sit idle before being closed. Default: 10 min"
                    .to_owned(),
            category: "sqlx_config".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(null),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(30),
                max: serde_json::json!(3600),
                step: Some(30.0),
            }),
            enum_options: None,
            units: Some("seconds".to_owned()),
            scientific_basis: None,
            env_variable: Some("SQLX_IDLE_TIMEOUT_SECS".to_owned()),
            is_runtime_configurable: false,
            requires_restart: true,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "sqlx.max_lifetime_secs".to_owned(),
            display_name: "Max Lifetime".to_owned(),
            description: "Maximum lifetime of a connection before it is closed. Default: 30 min"
                .to_owned(),
            category: "sqlx_config".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(null),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(60),
                max: serde_json::json!(7200),
                step: Some(60.0),
            }),
            enum_options: None,
            units: Some("seconds".to_owned()),
            scientific_basis: None,
            env_variable: Some("SQLX_MAX_LIFETIME_SECS".to_owned()),
            is_runtime_configurable: false,
            requires_restart: true,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "sqlx.test_before_acquire".to_owned(),
            display_name: "Test Before Acquire".to_owned(),
            description: "Whether to test connections before acquiring from pool".to_owned(),
            category: "sqlx_config".to_owned(),
            data_type: ConfigDataType::Boolean,
            default_value: serde_json::json!(true),
            valid_range: None,
            enum_options: None,
            units: None,
            scientific_basis: None,
            env_variable: Some("SQLX_TEST_BEFORE_ACQUIRE".to_owned()),
            is_runtime_configurable: false,
            requires_restart: true,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "sqlx.statement_cache_capacity".to_owned(),
            display_name: "Statement Cache Capacity".to_owned(),
            description: "Number of prepared statements to cache per connection. Default: 100"
                .to_owned(),
            category: "sqlx_config".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(null),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(0),
                max: serde_json::json!(1000),
                step: Some(10.0),
            }),
            enum_options: None,
            units: Some("statements".to_owned()),
            scientific_basis: None,
            env_variable: Some("SQLX_STATEMENT_CACHE_CAPACITY".to_owned()),
            is_runtime_configurable: false,
            requires_restart: true,
        },
    );
}

/// Register the `Cache TTL Configuration` catalog entries.
pub(super) fn register_cache_ttl(defs: &mut HashMap<String, ParameterDefinition>) {
    // Cache TTL Configuration
    add_definition(
        defs,
        ParameterDefinition {
            key: "cache.profile_ttl_secs".to_owned(),
            display_name: "Profile Cache TTL".to_owned(),
            description: "Time-to-live for cached athlete profiles".to_owned(),
            category: "cache_ttl".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(3600),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(60),
                max: serde_json::json!(86400),
                step: Some(60.0),
            }),
            enum_options: None,
            units: Some("seconds".to_owned()),
            scientific_basis: None,
            env_variable: Some("CACHE_PROFILE_TTL_SECS".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "cache.activity_list_ttl_secs".to_owned(),
            display_name: "Activity List Cache TTL".to_owned(),
            description: "Time-to-live for cached activity lists".to_owned(),
            category: "cache_ttl".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(300),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(30),
                max: serde_json::json!(3600),
                step: Some(30.0),
            }),
            enum_options: None,
            units: Some("seconds".to_owned()),
            scientific_basis: None,
            env_variable: Some("CACHE_ACTIVITY_LIST_TTL_SECS".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "cache.activity_ttl_secs".to_owned(),
            display_name: "Activity Cache TTL".to_owned(),
            description: "Time-to-live for cached individual activities".to_owned(),
            category: "cache_ttl".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(1800),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(60),
                max: serde_json::json!(7200),
                step: Some(60.0),
            }),
            enum_options: None,
            units: Some("seconds".to_owned()),
            scientific_basis: None,
            env_variable: Some("CACHE_ACTIVITY_TTL_SECS".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "cache.stats_ttl_secs".to_owned(),
            display_name: "Stats Cache TTL".to_owned(),
            description: "Time-to-live for cached athlete statistics".to_owned(),
            category: "cache_ttl".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(3600),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(300),
                max: serde_json::json!(86400),
                step: Some(300.0),
            }),
            enum_options: None,
            units: Some("seconds".to_owned()),
            scientific_basis: None,
            env_variable: Some("CACHE_STATS_TTL_SECS".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}

/// Register the `Strava Provider Settings` catalog entries.
pub(super) fn register_strava_provider(defs: &mut HashMap<String, ParameterDefinition>) {
    // Strava Provider Settings
    add_definition(
        defs,
        ParameterDefinition {
            key: "provider.strava_rate_limit_15min".to_owned(),
            display_name: "Strava 15-Min Rate Limit".to_owned(),
            description: "Maximum Strava API requests per 15 minutes".to_owned(),
            category: "provider_strava".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(100),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(10),
                max: serde_json::json!(1000),
                step: Some(10.0),
            }),
            enum_options: None,
            units: Some("requests".to_owned()),
            scientific_basis: Some("Strava API documentation".to_owned()),
            env_variable: Some("STRAVA_RATE_LIMIT_15MIN".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "provider.strava_rate_limit_daily".to_owned(),
            display_name: "Strava Daily Rate Limit".to_owned(),
            description: "Maximum Strava API requests per day".to_owned(),
            category: "provider_strava".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(1000),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(100),
                max: serde_json::json!(10000),
                step: Some(100.0),
            }),
            enum_options: None,
            units: Some("requests".to_owned()),
            scientific_basis: Some("Strava API documentation".to_owned()),
            env_variable: Some("STRAVA_RATE_LIMIT_DAILY".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "provider.strava_default_activities_per_page".to_owned(),
            display_name: "Strava Default Page Size".to_owned(),
            description: "Default number of activities per API page request".to_owned(),
            category: "provider_strava".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(30),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(1),
                max: serde_json::json!(200),
                step: Some(1.0),
            }),
            enum_options: None,
            units: Some("activities".to_owned()),
            scientific_basis: None,
            env_variable: Some("STRAVA_DEFAULT_ACTIVITIES_PER_PAGE".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "provider.strava_max_activities_per_request".to_owned(),
            display_name: "Strava Max Activities Per Request".to_owned(),
            description: "Maximum activities allowed in a single request".to_owned(),
            category: "provider_strava".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(200),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(10),
                max: serde_json::json!(500),
                step: Some(10.0),
            }),
            enum_options: None,
            units: Some("activities".to_owned()),
            scientific_basis: None,
            env_variable: Some("STRAVA_MAX_ACTIVITIES_PER_REQUEST".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}

/// Register the `Fitbit Provider Settings` catalog entries.
pub(super) fn register_fitbit_provider(defs: &mut HashMap<String, ParameterDefinition>) {
    // Fitbit Provider Settings
    add_definition(
        defs,
        ParameterDefinition {
            key: "provider.fitbit_rate_limit_hourly".to_owned(),
            display_name: "Fitbit Hourly Rate Limit".to_owned(),
            description: "Maximum Fitbit API requests per hour".to_owned(),
            category: "provider_fitbit".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(150),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(10),
                max: serde_json::json!(500),
                step: Some(10.0),
            }),
            enum_options: None,
            units: Some("requests".to_owned()),
            scientific_basis: Some("Fitbit API documentation".to_owned()),
            env_variable: Some("FITBIT_RATE_LIMIT_HOURLY".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "provider.fitbit_rate_limit_daily".to_owned(),
            display_name: "Fitbit Daily Rate Limit".to_owned(),
            description: "Maximum Fitbit API requests per day".to_owned(),
            category: "provider_fitbit".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(2000),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(100),
                max: serde_json::json!(10000),
                step: Some(100.0),
            }),
            enum_options: None,
            units: Some("requests".to_owned()),
            scientific_basis: Some("Fitbit API documentation".to_owned()),
            env_variable: Some("FITBIT_RATE_LIMIT_DAILY".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}

/// Register the `Garmin Provider Settings` catalog entries.
pub(super) fn register_garmin_provider(defs: &mut HashMap<String, ParameterDefinition>) {
    // Garmin Provider Settings
    add_definition(
        defs,
        ParameterDefinition {
            key: "provider.garmin_rate_limit_hourly".to_owned(),
            display_name: "Garmin Hourly Rate Limit".to_owned(),
            description: "Maximum Garmin API requests per hour".to_owned(),
            category: "provider_garmin".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(100),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(10),
                max: serde_json::json!(500),
                step: Some(10.0),
            }),
            enum_options: None,
            units: Some("requests".to_owned()),
            scientific_basis: Some("Garmin API documentation".to_owned()),
            env_variable: Some("GARMIN_RATE_LIMIT_HOURLY".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "provider.garmin_rate_limit_daily".to_owned(),
            display_name: "Garmin Daily Rate Limit".to_owned(),
            description: "Maximum Garmin API requests per day".to_owned(),
            category: "provider_garmin".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(1000),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(100),
                max: serde_json::json!(5000),
                step: Some(100.0),
            }),
            enum_options: None,
            units: Some("requests".to_owned()),
            scientific_basis: Some("Garmin API documentation".to_owned()),
            env_variable: Some("GARMIN_RATE_LIMIT_DAILY".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "provider.garmin_default_activities_per_page".to_owned(),
            display_name: "Garmin Default Page Size".to_owned(),
            description: "Default number of activities per API page request".to_owned(),
            category: "provider_garmin".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(20),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(1),
                max: serde_json::json!(100),
                step: Some(1.0),
            }),
            enum_options: None,
            units: Some("activities".to_owned()),
            scientific_basis: None,
            env_variable: Some("GARMIN_DEFAULT_ACTIVITIES_PER_PAGE".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "provider.garmin_max_activities_per_request".to_owned(),
            display_name: "Garmin Max Activities Per Request".to_owned(),
            description: "Maximum activities allowed in a single request".to_owned(),
            category: "provider_garmin".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(100),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(10),
                max: serde_json::json!(200),
                step: Some(10.0),
            }),
            enum_options: None,
            units: Some("activities".to_owned()),
            scientific_basis: None,
            env_variable: Some("GARMIN_MAX_ACTIVITIES_PER_REQUEST".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "provider.garmin_rate_limit_block_secs".to_owned(),
            display_name: "Garmin Rate Limit Block Duration".to_owned(),
            description: "Estimated block duration when rate limited".to_owned(),
            category: "provider_garmin".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(3600),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(60),
                max: serde_json::json!(86400),
                step: Some(60.0),
            }),
            enum_options: None,
            units: Some("seconds".to_owned()),
            scientific_basis: None,
            env_variable: Some("GARMIN_RATE_LIMIT_BLOCK_SECS".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}

/// Register the `MCP Network Settings` catalog entries.
pub(super) fn register_mcp_network(defs: &mut HashMap<String, ParameterDefinition>) {
    // MCP Network Settings
    add_definition(
        defs,
        ParameterDefinition {
            key: "mcp.max_request_size".to_owned(),
            display_name: "Max Request Size".to_owned(),
            description: "Maximum size for incoming MCP requests".to_owned(),
            category: "mcp_network".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(16_777_216),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(1_048_576),
                max: serde_json::json!(104_857_600),
                step: Some(1_048_576.0),
            }),
            enum_options: None,
            units: Some("bytes".to_owned()),
            scientific_basis: None,
            env_variable: Some("MCP_MAX_REQUEST_SIZE".to_owned()),
            is_runtime_configurable: false,
            requires_restart: true,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "mcp.max_response_size".to_owned(),
            display_name: "Max Response Size".to_owned(),
            description: "Maximum size for outgoing MCP responses".to_owned(),
            category: "mcp_network".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(67_108_864),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(1_048_576),
                max: serde_json::json!(268_435_456),
                step: Some(1_048_576.0),
            }),
            enum_options: None,
            units: Some("bytes".to_owned()),
            scientific_basis: None,
            env_variable: Some("MCP_MAX_RESPONSE_SIZE".to_owned()),
            is_runtime_configurable: false,
            requires_restart: true,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "mcp.notification_channel_size".to_owned(),
            display_name: "Notification Channel Size".to_owned(),
            description: "Buffer size for notification channels".to_owned(),
            category: "mcp_network".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(100),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(10),
                max: serde_json::json!(1000),
                step: Some(10.0),
            }),
            enum_options: None,
            units: Some("messages".to_owned()),
            scientific_basis: None,
            env_variable: Some("MCP_NOTIFICATION_CHANNEL_SIZE".to_owned()),
            is_runtime_configurable: false,
            requires_restart: true,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "mcp.websocket_channel_capacity".to_owned(),
            display_name: "WebSocket Channel Capacity".to_owned(),
            description: "Buffer capacity for WebSocket message channels".to_owned(),
            category: "mcp_network".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(256),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(32),
                max: serde_json::json!(2048),
                step: Some(32.0),
            }),
            enum_options: None,
            units: Some("messages".to_owned()),
            scientific_basis: None,
            env_variable: Some("MCP_WEBSOCKET_CHANNEL_CAPACITY".to_owned()),
            is_runtime_configurable: false,
            requires_restart: true,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "mcp.tcp_keep_alive_secs".to_owned(),
            display_name: "TCP Keep-Alive Interval".to_owned(),
            description: "TCP keep-alive interval for connections".to_owned(),
            category: "mcp_network".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(30),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(10),
                max: serde_json::json!(300),
                step: Some(10.0),
            }),
            enum_options: None,
            units: Some("seconds".to_owned()),
            scientific_basis: None,
            env_variable: Some("MCP_TCP_KEEP_ALIVE_SECS".to_owned()),
            is_runtime_configurable: false,
            requires_restart: true,
        },
    );
}

/// Register the `Monitoring Thresholds` catalog entries.
pub(super) fn register_monitoring(defs: &mut HashMap<String, ParameterDefinition>) {
    // Monitoring Thresholds
    add_definition(
        defs,
        ParameterDefinition {
            key: "monitoring.memory_warn_mb".to_owned(),
            display_name: "Memory Warning Threshold".to_owned(),
            description: "Memory usage threshold for warning alerts".to_owned(),
            category: "monitoring".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(512),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(128),
                max: serde_json::json!(4096),
                step: Some(64.0),
            }),
            enum_options: None,
            units: Some("MB".to_owned()),
            scientific_basis: None,
            env_variable: Some("MONITORING_MEMORY_WARN_MB".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "monitoring.memory_critical_mb".to_owned(),
            display_name: "Memory Critical Threshold".to_owned(),
            description: "Memory usage threshold for critical alerts".to_owned(),
            category: "monitoring".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(1024),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(256),
                max: serde_json::json!(8192),
                step: Some(128.0),
            }),
            enum_options: None,
            units: Some("MB".to_owned()),
            scientific_basis: None,
            env_variable: Some("MONITORING_MEMORY_CRITICAL_MB".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "monitoring.latency_warn_ms".to_owned(),
            display_name: "Latency Warning Threshold".to_owned(),
            description: "Request latency threshold for warning alerts".to_owned(),
            category: "monitoring".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(500),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(100),
                max: serde_json::json!(5000),
                step: Some(100.0),
            }),
            enum_options: None,
            units: Some("ms".to_owned()),
            scientific_basis: None,
            env_variable: Some("MONITORING_LATENCY_WARN_MS".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "monitoring.latency_critical_ms".to_owned(),
            display_name: "Latency Critical Threshold".to_owned(),
            description: "Request latency threshold for critical alerts".to_owned(),
            category: "monitoring".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(2000),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(500),
                max: serde_json::json!(30000),
                step: Some(500.0),
            }),
            enum_options: None,
            units: Some("ms".to_owned()),
            scientific_basis: None,
            env_variable: Some("MONITORING_LATENCY_CRITICAL_MS".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "monitoring.error_rate_warn_pct".to_owned(),
            display_name: "Error Rate Warning Threshold".to_owned(),
            description: "Error rate percentage for warning alerts".to_owned(),
            category: "monitoring".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(1.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(0.1),
                max: serde_json::json!(10.0),
                step: Some(0.1),
            }),
            enum_options: None,
            units: Some("%".to_owned()),
            scientific_basis: None,
            env_variable: Some("MONITORING_ERROR_RATE_WARN_PCT".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "monitoring.error_rate_critical_pct".to_owned(),
            display_name: "Error Rate Critical Threshold".to_owned(),
            description: "Error rate percentage for critical alerts".to_owned(),
            category: "monitoring".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(5.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(1.0),
                max: serde_json::json!(25.0),
                step: Some(0.5),
            }),
            enum_options: None,
            units: Some("%".to_owned()),
            scientific_basis: None,
            env_variable: Some("MONITORING_ERROR_RATE_CRITICAL_PCT".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "monitoring.pool_usage_warn_pct".to_owned(),
            display_name: "Connection Pool Warning Threshold".to_owned(),
            description: "Connection pool usage percentage for warning alerts".to_owned(),
            category: "monitoring".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(70.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(50.0),
                max: serde_json::json!(90.0),
                step: Some(5.0),
            }),
            enum_options: None,
            units: Some("%".to_owned()),
            scientific_basis: None,
            env_variable: Some("MONITORING_POOL_USAGE_WARN_PCT".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "monitoring.pool_usage_critical_pct".to_owned(),
            display_name: "Connection Pool Critical Threshold".to_owned(),
            description: "Connection pool usage percentage for critical alerts".to_owned(),
            category: "monitoring".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(90.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(70.0),
                max: serde_json::json!(99.0),
                step: Some(5.0),
            }),
            enum_options: None,
            units: Some("%".to_owned()),
            scientific_basis: None,
            env_variable: Some("MONITORING_POOL_USAGE_CRITICAL_PCT".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "monitoring.cache_hit_rate_warn_pct".to_owned(),
            display_name: "Cache Hit Rate Warning Threshold".to_owned(),
            description: "Cache hit rate percentage below which warning alerts are triggered"
                .to_owned(),
            category: "monitoring".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(80.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(50.0),
                max: serde_json::json!(95.0),
                step: Some(5.0),
            }),
            enum_options: None,
            units: Some("%".to_owned()),
            scientific_basis: None,
            env_variable: Some("MONITORING_CACHE_HIT_RATE_WARN_PCT".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "monitoring.cache_miss_rate_critical_pct".to_owned(),
            display_name: "Cache Miss Rate Critical Threshold".to_owned(),
            description: "Cache miss rate percentage above which critical alerts are triggered"
                .to_owned(),
            category: "monitoring".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(50.0),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(20.0),
                max: serde_json::json!(80.0),
                step: Some(5.0),
            }),
            enum_options: None,
            units: Some("%".to_owned()),
            scientific_basis: None,
            env_variable: Some("MONITORING_CACHE_MISS_RATE_CRITICAL_PCT".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}

/// Register the `Usage Quotas — per-user and per-tenant limits` catalog entries.
pub(super) fn register_usage_quotas(defs: &mut HashMap<String, ParameterDefinition>) {
    // Usage Quotas — per-user and per-tenant limits
    add_definition(
        defs,
        ParameterDefinition {
            key: "usage_quotas.max_coaches_per_user".to_owned(),
            display_name: "Max Coaches Per User".to_owned(),
            description: "Maximum number of coaches a single user can subscribe to".to_owned(),
            category: "usage_quotas".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(3),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(1),
                max: serde_json::json!(20),
                step: Some(1.0),
            }),
            enum_options: None,
            units: Some("coaches".to_owned()),
            scientific_basis: None,
            env_variable: Some("QUOTA_MAX_COACHES_PER_USER".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "usage_quotas.max_active_conversations".to_owned(),
            display_name: "Max Active Conversations".to_owned(),
            description: "Maximum concurrent active conversations per user".to_owned(),
            category: "usage_quotas".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(10),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(1),
                max: serde_json::json!(50),
                step: Some(1.0),
            }),
            enum_options: None,
            units: Some("conversations".to_owned()),
            scientific_basis: None,
            env_variable: Some("QUOTA_MAX_ACTIVE_CONVERSATIONS".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "usage_quotas.daily_message_cap".to_owned(),
            display_name: "Daily Message Cap".to_owned(),
            description: "Maximum chat messages a user can send per day".to_owned(),
            category: "usage_quotas".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(50),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(5),
                max: serde_json::json!(1000),
                step: Some(5.0),
            }),
            enum_options: None,
            units: Some("messages/day".to_owned()),
            scientific_basis: None,
            env_variable: Some("QUOTA_DAILY_MESSAGE_CAP".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "usage_quotas.weekly_message_cap".to_owned(),
            display_name: "Weekly Message Cap".to_owned(),
            description: "Maximum chat messages a user can send per week".to_owned(),
            category: "usage_quotas".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(250),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(25),
                max: serde_json::json!(5000),
                step: Some(25.0),
            }),
            enum_options: None,
            units: Some("messages/week".to_owned()),
            scientific_basis: None,
            env_variable: Some("QUOTA_WEEKLY_MESSAGE_CAP".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "usage_quotas.daily_tool_call_limit".to_owned(),
            display_name: "Daily Tool Call Limit".to_owned(),
            description: "Maximum MCP tool calls a user can trigger per day".to_owned(),
            category: "usage_quotas".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(100),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(10),
                max: serde_json::json!(5000),
                step: Some(10.0),
            }),
            enum_options: None,
            units: Some("calls/day".to_owned()),
            scientific_basis: None,
            env_variable: Some("QUOTA_DAILY_TOOL_CALL_LIMIT".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "usage_quotas.weekly_tool_call_limit".to_owned(),
            display_name: "Weekly Tool Call Limit".to_owned(),
            description: "Maximum MCP tool calls a user can trigger per week".to_owned(),
            category: "usage_quotas".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(500),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(50),
                max: serde_json::json!(25000),
                step: Some(50.0),
            }),
            enum_options: None,
            units: Some("calls/week".to_owned()),
            scientific_basis: None,
            env_variable: Some("QUOTA_WEEKLY_TOOL_CALL_LIMIT".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "usage_quotas.daily_token_budget".to_owned(),
            display_name: "Daily Token Budget".to_owned(),
            description: "Maximum total LLM tokens (prompt + completion) per user per day"
                .to_owned(),
            category: "usage_quotas".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(500_000),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(10_000),
                max: serde_json::json!(10_000_000),
                step: Some(10_000.0),
            }),
            enum_options: None,
            units: Some("tokens/day".to_owned()),
            scientific_basis: None,
            env_variable: Some("QUOTA_DAILY_TOKEN_BUDGET".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "usage_quotas.weekly_token_budget".to_owned(),
            display_name: "Weekly Token Budget".to_owned(),
            description: "Maximum total LLM tokens (prompt + completion) per user per week"
                .to_owned(),
            category: "usage_quotas".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(2_000_000),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(50_000),
                max: serde_json::json!(50_000_000),
                step: Some(50_000.0),
            }),
            enum_options: None,
            units: Some("tokens/week".to_owned()),
            scientific_basis: None,
            env_variable: Some("QUOTA_WEEKLY_TOKEN_BUDGET".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "usage_quotas.burst_multiplier".to_owned(),
            display_name: "Burst Multiplier".to_owned(),
            description:
                "Multiplier applied to daily limits during burst periods (e.g. 1.5 = 50% extra)"
                    .to_owned(),
            category: "usage_quotas".to_owned(),
            data_type: ConfigDataType::Float,
            default_value: serde_json::json!(1.5),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(1.0),
                max: serde_json::json!(5.0),
                step: Some(0.1),
            }),
            enum_options: None,
            units: Some("x".to_owned()),
            scientific_basis: None,
            env_variable: Some("QUOTA_BURST_MULTIPLIER".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "usage_quotas.warning_threshold_percent".to_owned(),
            display_name: "Warning Threshold".to_owned(),
            description:
                "Percentage of quota usage at which the user receives a warning notification"
                    .to_owned(),
            category: "usage_quotas".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(80),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(50),
                max: serde_json::json!(99),
                step: Some(1.0),
            }),
            enum_options: None,
            units: Some("%".to_owned()),
            scientific_basis: None,
            env_variable: Some("QUOTA_WARNING_THRESHOLD_PERCENT".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}

/// Register the `Activity access quotas — separate limits for summary vs detailed mode` catalog entries.
pub(super) fn register_activity_access_quotas(defs: &mut HashMap<String, ParameterDefinition>) {
    // Activity access quotas — separate limits for summary vs detailed mode
    add_definition(
        defs,
        ParameterDefinition {
            key: "usage_quotas.daily_activity_summary_limit".to_owned(),
            display_name: "Daily Activity Summary Limit".to_owned(),
            description: "Maximum activity summary requests per coach per day".to_owned(),
            category: "usage_quotas".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(100),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(10),
                max: serde_json::json!(1000),
                step: Some(10.0),
            }),
            enum_options: None,
            units: Some("requests/day".to_owned()),
            scientific_basis: None,
            env_variable: Some("QUOTA_DAILY_ACTIVITY_SUMMARY_LIMIT".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "usage_quotas.weekly_activity_summary_limit".to_owned(),
            display_name: "Weekly Activity Summary Limit".to_owned(),
            description: "Maximum activity summary requests per coach per week".to_owned(),
            category: "usage_quotas".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(500),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(50),
                max: serde_json::json!(5000),
                step: Some(50.0),
            }),
            enum_options: None,
            units: Some("requests/week".to_owned()),
            scientific_basis: None,
            env_variable: Some("QUOTA_WEEKLY_ACTIVITY_SUMMARY_LIMIT".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "usage_quotas.daily_activity_detailed_limit".to_owned(),
            display_name: "Daily Activity Detailed Limit".to_owned(),
            description: "Maximum detailed activity requests per coach per day (higher token cost)"
                .to_owned(),
            category: "usage_quotas".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(20),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(5),
                max: serde_json::json!(500),
                step: Some(5.0),
            }),
            enum_options: None,
            units: Some("requests/day".to_owned()),
            scientific_basis: None,
            env_variable: Some("QUOTA_DAILY_ACTIVITY_DETAILED_LIMIT".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "usage_quotas.weekly_activity_detailed_limit".to_owned(),
            display_name: "Weekly Activity Detailed Limit".to_owned(),
            description:
                "Maximum detailed activity requests per coach per week (higher token cost)"
                    .to_owned(),
            category: "usage_quotas".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(100),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(25),
                max: serde_json::json!(2500),
                step: Some(25.0),
            }),
            enum_options: None,
            units: Some("requests/week".to_owned()),
            scientific_basis: None,
            env_variable: Some("QUOTA_WEEKLY_ACTIVITY_DETAILED_LIMIT".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );

    add_definition(
        defs,
        ParameterDefinition {
            key: "usage_quotas.counter_retention_days".to_owned(),
            display_name: "Counter Retention Days".to_owned(),
            description: "Number of days to retain old usage counter records before pruning"
                .to_owned(),
            category: "usage_quotas".to_owned(),
            data_type: ConfigDataType::Integer,
            default_value: serde_json::json!(90),
            valid_range: Some(ParameterRange {
                min: serde_json::json!(7),
                max: serde_json::json!(365),
                step: Some(1.0),
            }),
            enum_options: None,
            units: Some("days".to_owned()),
            scientific_basis: None,
            env_variable: Some("QUOTA_COUNTER_RETENTION_DAYS".to_owned()),
            is_runtime_configurable: true,
            requires_restart: false,
        },
    );
}
