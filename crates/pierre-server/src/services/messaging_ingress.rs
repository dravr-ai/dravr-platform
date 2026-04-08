// ABOUTME: Business logic for messaging ingress: OTP flow, channel linking, session resolution,
// ABOUTME: slash command dispatch, message persistence, LLM dispatch, and outbound response handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{Duration, Utc};
use pierre_core::models::messaging::{
    CardAction, ChannelConfig, ChannelType, IncomingMessage, MessageContent, OutgoingMessage,
    LINK_CODE_TTL_MINUTES, MAX_OTP_ATTEMPTS, OTP_TTL_MINUTES,
};
use pierre_core::models::usage::InsertLlmUsage;
use pierre_core::models::{TenantId, User};
use pierre_database::plugins::{
    CreateChannelLinkParams, CreateLinkStateParams, CreateSessionParams, InsertMessageParams,
    MessagingRepository, TenantRepository, UserRepository,
};
use pierre_llm::pricing::estimate_tokens;
use pierre_llm::TokenUsage;
use pierre_messaging::channel::MessagingChannel;
use rand::Rng;
use sha2::{Digest, Sha256};
use std::env;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::errors::AppError;
use crate::mcp::resources::ServerResources;
use crate::routes::messaging::linking::generate_link_code;
use crate::services::analytics::{analytics, hash_id};
use crate::services::chat_orchestration;
use crate::services::usage_counter::UsageCounterService;
