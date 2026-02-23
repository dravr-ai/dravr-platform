// ABOUTME: Comprehensive token consumption analysis tests for DRAVR-419/420/417 close-out
// ABOUTME: Validates schema token estimates, TOON efficiency, prompt sizes, and cost projections
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_mcp_server::formatters::{format_output, OutputFormat, TokenEfficiencyMetrics};
use pierre_mcp_server::llm::pricing::calculate_cost;
use pierre_mcp_server::llm::prompts::{
    COACH_GENERATION_PROMPT, INSIGHT_GENERATION_PROMPT, INSIGHT_VALIDATION_PROMPT,
    PIERRE_SYSTEM_PROMPT,
};
use pierre_mcp_server::tools::ToolRegistry;
use serde::Serialize;

#[derive(Serialize, Clone)]
struct ActivitySample {
    id: String,
    name: String,
    sport_type: String,
    distance_meters: f64,
    moving_time_seconds: u32,
    elapsed_time_seconds: u32,
    total_elevation_gain: f64,
    start_date: String,
    average_speed: f64,
    max_speed: f64,
    average_heartrate: Option<f64>,
    max_heartrate: Option<f64>,
    suffer_score: Option<f64>,
    calories: Option<f64>,
}

fn create_realistic_activities(count: usize) -> Vec<ActivitySample> {
    (0..count)
        .map(|i| {
            let day = i % 28 + 1;
            let month = (i / 28) % 12 + 1;
            ActivitySample {
                id: format!("activity_{}", 12_345_678 + i),
                name: match i % 5 {
                    0 => format!("Morning Run #{}", i + 1),
                    1 => format!("Lunch Ride #{}", i + 1),
                    2 => format!("Evening Swim #{}", i + 1),
                    3 => format!("Trail Run #{}", i + 1),
                    _ => format!("Recovery Walk #{}", i + 1),
                },
                sport_type: match i % 5 {
                    0 | 3 => "Run".to_owned(),
                    1 => "Ride".to_owned(),
                    2 => "Swim".to_owned(),
                    _ => "Walk".to_owned(),
                },
                distance_meters: (i as f64).mul_add(150.0, 5000.0),
                moving_time_seconds: 1800 + (i as u32 * 45),
                elapsed_time_seconds: 2000 + (i as u32 * 50),
                total_elevation_gain: (i as f64) * 12.5,
                start_date: format!("2026-{month:02}-{day:02}T07:30:00Z"),
                average_speed: 2.5 + (i as f64 * 0.05),
                max_speed: 4.0 + (i as f64 * 0.08),
                average_heartrate: if i % 3 == 0 {
                    None
                } else {
                    Some(145.0 + i as f64)
                },
                max_heartrate: if i % 3 == 0 {
                    None
                } else {
                    Some(172.0 + i as f64)
                },
                suffer_score: if i % 4 == 0 {
                    None
                } else {
                    Some(50.0 + i as f64 * 2.0)
                },
                calories: Some(300.0 + i as f64 * 15.0),
            }
        })
        .collect()
}

mod axis1_schema_tokens {
    use super::*;

    #[test]
    fn test_tool_schema_total_token_estimate() {
        let mut registry = ToolRegistry::new();
        registry.register_builtin_tools();
        let estimate = registry.total_schema_token_estimate();

        println!("\n=== AXIS 1: Tool Schema Token Budget (DRAVR-419) ===");
        println!("Total tools registered: {}", estimate.tool_count);
        println!("Total schema bytes:     {}", estimate.total_bytes);
        println!("Estimated tokens:       {}", estimate.estimated_tokens);
        println!();
        println!("Top 10 tools by token cost:");
        println!("{:<40} {:>8} {:>8}", "Tool", "Bytes", "Tokens");
        println!("{}", "-".repeat(58));
        for tool in estimate.per_tool.iter().take(10) {
            println!("{:<40} {:>8} {:>8}", tool.name, tool.bytes, tool.tokens);
        }
        println!();

        assert!(estimate.tool_count > 0, "Should have registered tools");
        assert!(estimate.total_bytes > 0, "Total bytes should be non-zero");
        assert!(
            estimate.estimated_tokens > 0,
            "Estimated tokens should be non-zero"
        );
        assert_eq!(
            estimate.per_tool.len(),
            estimate.tool_count,
            "Per-tool breakdown should match tool count"
        );
        // Verify sorted descending by tokens
        assert!(
            estimate
                .per_tool
                .windows(2)
                .all(|w| w[0].tokens >= w[1].tokens),
            "Per-tool list should be sorted descending by tokens"
        );
    }
}

mod axis1_toon_efficiency {
    use super::*;

    #[test]
    fn test_toon_vs_json_savings_with_realistic_data() {
        println!("\n=== AXIS 1: TOON vs JSON Savings (DRAVR-419) ===");
        println!(
            "{:<12} {:>12} {:>12} {:>9} {:>13} {:>13}",
            "Activities", "JSON bytes", "TOON bytes", "Savings%", "JSON tokens", "TOON tokens"
        );
        println!("{}", "-".repeat(75));

        for &count in &[5, 10, 25, 50, 100] {
            let activities = create_realistic_activities(count);
            let json_output = format_output(&activities, OutputFormat::Json).unwrap();
            let toon_output = format_output(&activities, OutputFormat::Toon).unwrap();
            let json_metrics = json_output.calculate_efficiency(&activities);
            let toon_metrics = toon_output.calculate_efficiency(&activities);

            println!(
                "{:<12} {:>12} {:>12} {:>8.1}% {:>13} {:>13}",
                count,
                json_metrics.byte_size,
                toon_metrics.byte_size,
                toon_metrics.token_savings_percent,
                json_metrics.estimated_tokens,
                toon_metrics.estimated_tokens,
            );

            assert!(
                toon_metrics.byte_size <= json_metrics.byte_size,
                "TOON should be <= JSON for {count} activities: TOON={}, JSON={}",
                toon_metrics.byte_size,
                json_metrics.byte_size
            );
        }

        // Detailed 50-activity case
        let activities_50 = create_realistic_activities(50);
        let json_50 = format_output(&activities_50, OutputFormat::Json).unwrap();
        let toon_50 = format_output(&activities_50, OutputFormat::Toon).unwrap();
        let json_m = json_50.calculate_efficiency(&activities_50);
        let toon_m = toon_50.calculate_efficiency(&activities_50);

        println!();
        println!("--- Detailed: 50 activities ---");
        println!(
            "JSON: {} bytes, {} tokens",
            json_m.byte_size, json_m.estimated_tokens
        );
        println!(
            "TOON: {} bytes, {} tokens",
            toon_m.byte_size, toon_m.estimated_tokens
        );
        println!(
            "Savings: {:.1}% bytes, compression ratio {:.2}x",
            toon_m.token_savings_percent, toon_m.compression_ratio
        );
    }

    #[test]
    fn test_tool_response_size_samples() {
        println!("\n=== AXIS 1: Tool Response Size Samples (DRAVR-419) ===");
        println!("{:<20} {:>12} {:>12}", "Scenario", "Bytes", "Est. Tokens");
        println!("{}", "-".repeat(46));

        let mut prev_tokens = 0;
        for &days in &[30, 90, 365] {
            let activities = create_realistic_activities(days);
            let json = serde_json::to_string(&activities).unwrap();
            let tokens = TokenEfficiencyMetrics::estimate_tokens(&json);
            println!(
                "{:<20} {:>12} {:>12}",
                format!("get_activities({days}d)"),
                json.len(),
                tokens
            );
            assert!(tokens > 0, "Tokens should be > 0 for {days} days");
            if prev_tokens > 0 {
                assert!(
                    tokens > prev_tokens,
                    "More days should yield more tokens: {days}d={tokens} vs prev={prev_tokens}"
                );
            }
            prev_tokens = tokens;
        }

        // Small stats object
        #[derive(Serialize)]
        struct Stats {
            total_distance: f64,
            total_duration: u32,
            activity_count: u32,
            avg_heartrate: f64,
            max_heartrate: f64,
            total_elevation: f64,
            weekly_volume: f64,
        }
        let stats = Stats {
            total_distance: 245_000.0,
            total_duration: 86_400,
            activity_count: 30,
            avg_heartrate: 152.3,
            max_heartrate: 188.0,
            total_elevation: 3500.0,
            weekly_volume: 45.2,
        };
        let stats_json = serde_json::to_string(&stats).unwrap();
        let stats_tokens = TokenEfficiencyMetrics::estimate_tokens(&stats_json);
        println!(
            "{:<20} {:>12} {:>12}",
            "stats_summary",
            stats_json.len(),
            stats_tokens
        );
        assert!(
            stats_tokens < prev_tokens,
            "Stats object should be smaller than 365-day activities"
        );
    }
}

mod axis2_prompt_sizes {
    use super::*;

    #[test]
    fn test_static_prompt_token_sizes() {
        let prompts: &[(&str, &str)] = &[
            ("PIERRE_SYSTEM_PROMPT", PIERRE_SYSTEM_PROMPT),
            ("COACH_GENERATION_PROMPT", COACH_GENERATION_PROMPT),
            ("INSIGHT_VALIDATION_PROMPT", INSIGHT_VALIDATION_PROMPT),
            ("INSIGHT_GENERATION_PROMPT", INSIGHT_GENERATION_PROMPT),
        ];

        println!("\n=== AXIS 2: Static Prompt Token Sizes (DRAVR-420) ===");
        println!("{:<30} {:>10} {:>12}", "Prompt", "Bytes", "Est. Tokens");
        println!("{}", "-".repeat(54));

        let mut total_bytes = 0;
        let mut total_tokens = 0;
        let mut system_tokens = 0;

        for (name, content) in prompts {
            let bytes = content.len();
            let tokens = TokenEfficiencyMetrics::estimate_tokens(content);
            println!("{:<30} {:>10} {:>12}", name, bytes, tokens);
            total_bytes += bytes;
            total_tokens += tokens;
            if *name == "PIERRE_SYSTEM_PROMPT" {
                system_tokens = tokens;
            }

            assert!(!content.is_empty(), "{name} should not be empty");
            assert!(bytes > 100, "{name} should be > 100 bytes, got {bytes}");
        }

        let system_pct = system_tokens as f64 / total_tokens as f64 * 100.0;
        println!();
        println!("Total: {} bytes, {} tokens", total_bytes, total_tokens);
        println!(
            "System prompt is {:.1}% of total static prompt budget",
            system_pct
        );

        assert!(system_tokens > 0, "System prompt should have tokens");
        // System prompt should be the largest
        for (name, content) in prompts {
            let tokens = TokenEfficiencyMetrics::estimate_tokens(content);
            assert!(
                system_tokens >= tokens,
                "System prompt ({system_tokens}) should be >= {name} ({tokens})"
            );
        }
    }

    #[test]
    fn test_conversation_token_growth_projection() {
        let system_tokens = TokenEfficiencyMetrics::estimate_tokens(PIERRE_SYSTEM_PROMPT);
        // Per-turn token budget: user message + assistant response + tool call/response
        let user_tokens_per_turn: usize = 30;
        let assistant_tokens_per_turn: usize = 200;
        let tool_tokens_per_turn: usize = 500;
        let per_turn = user_tokens_per_turn + assistant_tokens_per_turn + tool_tokens_per_turn;

        println!("\n=== AXIS 2: Conversation Token Growth Projection (DRAVR-420) ===");
        println!("System prompt:     {} tokens", system_tokens);
        println!(
            "Per-turn budget:   {} tokens (user={}, assistant={}, tools={})",
            per_turn, user_tokens_per_turn, assistant_tokens_per_turn, tool_tokens_per_turn
        );
        println!();
        println!(
            "{:<8} {:>14} {:>14} {:>16} {:>16}",
            "Turns", "Total Billed", "Output Tokens", "Cost (3-flash)", "Cost (2.5-flash)"
        );
        println!("{}", "-".repeat(72));

        for &turns in &[1, 3, 5, 10, 20] {
            // Each turn re-sends the full context (system + all prior turns)
            let total_billed: usize = (1..=turns).map(|i| system_tokens + i * per_turn).sum();
            let total_output = turns * assistant_tokens_per_turn;

            let cost_3flash = calculate_cost(
                "gemini",
                "gemini-3-flash-preview",
                total_billed as i64,
                total_output as i64,
            );
            let cost_25flash = calculate_cost(
                "gemini",
                "gemini-2.5-flash",
                total_billed as i64,
                total_output as i64,
            );

            println!(
                "{:<8} {:>14} {:>14} {:>15.6} {:>15.6}",
                turns, total_billed, total_output, cost_3flash, cost_25flash
            );
        }

        assert!(
            system_tokens > 3000,
            "System prompt should be > 3000 tokens, got {system_tokens}"
        );
    }
}

mod axis2_cost_projections {
    use super::*;

    struct UserProfile {
        name: &'static str,
        chats_per_month: usize,
        turns_per_chat: usize,
    }

    #[test]
    fn test_per_user_monthly_cost_projection() {
        let system_tokens = TokenEfficiencyMetrics::estimate_tokens(PIERRE_SYSTEM_PROMPT);
        let per_turn: usize = 730; // user(30) + assistant(200) + tools(500)
        let assistant_per_turn: usize = 200;

        let profiles = [
            UserProfile {
                name: "Light",
                chats_per_month: 5,
                turns_per_chat: 3,
            },
            UserProfile {
                name: "Moderate",
                chats_per_month: 15,
                turns_per_chat: 5,
            },
            UserProfile {
                name: "Heavy",
                chats_per_month: 30,
                turns_per_chat: 8,
            },
            UserProfile {
                name: "Power",
                chats_per_month: 60,
                turns_per_chat: 10,
            },
        ];

        println!("\n=== AXIS 2: Per-User Monthly Cost Projection (DRAVR-417) ===");
        println!(
            "{:<10} {:>6} {:>6} {:>14} {:>14} {:>14} {:>14}",
            "Profile",
            "Chats",
            "Turns",
            "Input Tokens",
            "Output Tokens",
            "3-flash ($)",
            "2.5-flash ($)"
        );
        println!("{}", "-".repeat(84));

        for profile in &profiles {
            let mut total_input: usize = 0;
            let mut total_output: usize = 0;

            for _ in 0..profile.chats_per_month {
                for turn in 1..=profile.turns_per_chat {
                    total_input += system_tokens + turn * per_turn;
                    total_output += assistant_per_turn;
                }
            }

            let cost_3flash = calculate_cost(
                "gemini",
                "gemini-3-flash-preview",
                total_input as i64,
                total_output as i64,
            );
            let cost_25flash = calculate_cost(
                "gemini",
                "gemini-2.5-flash",
                total_input as i64,
                total_output as i64,
            );

            println!(
                "{:<10} {:>6} {:>6} {:>14} {:>14} {:>13.4} {:>13.4}",
                profile.name,
                profile.chats_per_month,
                profile.turns_per_chat,
                total_input,
                total_output,
                cost_3flash,
                cost_25flash,
            );

            assert!(
                cost_3flash > 0.0,
                "{} profile gemini-3-flash cost should be > 0",
                profile.name
            );
        }

        // Single operation costs
        let insight_tokens = TokenEfficiencyMetrics::estimate_tokens(INSIGHT_GENERATION_PROMPT);
        let coach_tokens = TokenEfficiencyMetrics::estimate_tokens(COACH_GENERATION_PROMPT);
        let insight_output: usize = 500;
        let coach_output: usize = 800;

        let insight_cost = calculate_cost(
            "gemini",
            "gemini-3-flash-preview",
            insight_tokens as i64,
            insight_output as i64,
        );
        let coach_cost = calculate_cost(
            "gemini",
            "gemini-3-flash-preview",
            coach_tokens as i64,
            coach_output as i64,
        );

        println!();
        println!("Single operation costs (gemini-3-flash):");
        println!(
            "  Insight generation: {} input tokens + {} output → ${:.6}",
            insight_tokens, insight_output, insight_cost
        );
        println!(
            "  Coach generation:   {} input tokens + {} output → ${:.6}",
            coach_tokens, coach_output, coach_cost
        );
    }

    #[test]
    fn test_cache_opportunity_analysis() {
        let system_tokens = TokenEfficiencyMetrics::estimate_tokens(PIERRE_SYSTEM_PROMPT);
        let activity_data_tokens: usize = 500;

        // Typical monthly usage: 75 conversations, each sends system prompt
        let system_prompt_sends: usize = 75;
        let redundant_system_tokens = system_tokens * (system_prompt_sends - 1);
        let system_waste_cost = calculate_cost(
            "gemini",
            "gemini-3-flash-preview",
            redundant_system_tokens as i64,
            0,
        );

        // Activity data fetched ~50 times per month via tools
        let activity_fetches: usize = 50;
        let redundant_activity_tokens = activity_data_tokens * (activity_fetches - 1);
        let activity_waste_cost = calculate_cost(
            "gemini",
            "gemini-3-flash-preview",
            redundant_activity_tokens as i64,
            0,
        );

        let total_monthly_waste = redundant_system_tokens + redundant_activity_tokens;

        println!("\n=== AXIS 2: Cache Opportunity Analysis (DRAVR-420) ===");
        println!(
            "System prompt: {} tokens, sent {} times/month",
            system_tokens, system_prompt_sends
        );
        println!(
            "  Redundant: {} tokens (74 extra sends) → ${:.6} waste",
            redundant_system_tokens, system_waste_cost
        );
        println!();
        println!(
            "Activity data: {} tokens/fetch, {} fetches/month",
            activity_data_tokens, activity_fetches
        );
        println!(
            "  Redundant: {} tokens (49 extra fetches) → ${:.6} waste",
            redundant_activity_tokens, activity_waste_cost
        );
        println!();
        println!("Total monthly token waste: {}", total_monthly_waste);
        println!(
            "Total monthly cost waste:  ${:.6}",
            system_waste_cost + activity_waste_cost
        );
        println!();
        println!("Batch API eligibility:");
        println!("  Insight generation: YES (async, no user interaction)");
        println!("  Coach generation:   YES (async, no user interaction)");
        println!("  Chat conversations: NO (interactive, latency-sensitive)");

        assert!(
            total_monthly_waste > 100_000,
            "Monthly token waste should exceed 100K tokens, got {total_monthly_waste}"
        );
    }
}
