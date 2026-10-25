// ABOUTME: The Intervals.icu provider factory — the registry's builder, and the coach-key builders fixed to one coached athlete
// ABOUTME: Whether a coach may read or write that athlete is decided before a delegated provider is built
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::IntervalsIcuProvider;
use crate::core::{FitnessProvider, ProviderConfig, ProviderFactory};
use crate::delegation::DelegatedReads;
use crate::errors::AppResult;
use crate::intervals_icu_roster::check_athlete_id;

/// Factory that builds [`IntervalsIcuProvider`] instances for the
/// [`ProviderRegistry`](crate::registry::ProviderRegistry).
#[derive(Debug)]
pub struct IntervalsIcuProviderFactory;

impl ProviderFactory for IntervalsIcuProviderFactory {
    fn create(&self, config: ProviderConfig) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(IntervalsIcuProvider::with_config(config)))
    }

    fn supported_providers(&self) -> &'static [&'static str] {
        &["intervals_icu"]
    }

    // A coach's API key reads every athlete who shares with the coach.
    fn delegated_reads(&self) -> Option<&dyn DelegatedReads> {
        Some(self)
    }
}

impl DelegatedReads for IntervalsIcuProviderFactory {
    fn check_athlete_id(&self, athlete_id: &str) -> AppResult<()> {
        check_athlete_id(athlete_id)
    }

    fn create_delegated(
        &self,
        config: ProviderConfig,
        athlete_id: &str,
    ) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(IntervalsIcuProvider::delegated(
            config, athlete_id,
        )?))
    }

    // A coach's API key writes the calendar of an athlete who made them
    // their coach.
    fn create_delegated_calendar_writer(
        &self,
        config: ProviderConfig,
        athlete_id: &str,
    ) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(IntervalsIcuProvider::delegated_calendar_writer(
            config, athlete_id,
        )?))
    }

    // A coach reading a coached athlete's calendar sees the events they keep
    // hidden from the athlete.
    fn create_delegated_coach_reader(
        &self,
        config: ProviderConfig,
        athlete_id: &str,
    ) -> AppResult<Box<dyn FitnessProvider>> {
        Ok(Box::new(IntervalsIcuProvider::delegated_coach_reader(
            config, athlete_id,
        )?))
    }
}
