// ABOUTME: Main entry point for the FitnessAnalysisAgent
// ABOUTME: Demonstrates autonomous A2A protocol usage for fitness data analysis
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use anyhow::Result;
use tracing::{info, error};

mod a2a_client;
mod analyzer;
mod config;
mod scheduler;

use crate::config::AgentConfig;
use crate::scheduler::AnalysisScheduler;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("fitness_analyzer=info")
        .init();

    info!("🤖 Starting Fitness Analysis Agent");
    info!("📡 Demonstrating A2A Protocol Integration");

    // Load configuration
    let config = AgentConfig::load()?;
    info!("⚙️ Configuration loaded: {} mode", 
        if config.development_mode { "development" } else { "production" });

    // Create and start the analysis scheduler
    let mut scheduler = AnalysisScheduler::new(config).await?;
    
    // Run the scheduler (this will loop indefinitely)
    match scheduler.run().await {
        Ok(()) => {
            info!("✅ Agent completed successfully");
            Ok(())
        }
        Err(e) => {
            error!("❌ Agent failed: {}", e);
            Err(e)
        }
    }
}