// ABOUTME: PostgreSQL-backed FuelCatalogueRepository and AthleteFuelItemRepository, emitted from the shared body in repositories/
// ABOUTME: Native uuid ids through the NativeUuid codec; the statements are the ones SQLite runs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    check_plausibility, fuel_natural_key, normalize_gtin, AthleteFuelItem, CatalogueFuelProduct,
    FactProvenance, FuelApprovalOutcome, FuelFacts, FuelItemEntry, FuelProductProposal,
    FuelProposalDetail, FuelRejectReason, NewFuelProposal, PendingFuelItem, ProposalSourceImage,
    ProposalStatus, TenantId, MAX_FUEL_ITEMS_PER_ATHLETE, MAX_FUEL_PRODUCT_LOOKUP,
};
use sqlx::postgres::PgRow;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::fuel_products::{
    catalogue_from_row, check_gtin, from_json, fuel_error, impl_fuel_product_repositories,
    merge_for_approval, parse_form, parse_status, proposal_from_row, search_patterns, to_json,
    AthleteFuelItemRepository, CatalogueIds, FuelCatalogueRepository, FuelIdentityKey, ProposalIds,
    CLAIM_PROPOSAL_SQL, COUNT_LIVE_ITEMS_SQL, COUNT_PRODUCT_SQL, COUNT_PROPOSALS_BY_HASH_SQL,
    FIND_LIVE_PENDING_ITEM_SQL, FIND_LIVE_PRODUCT_ITEM_SQL, GET_PRODUCT_BY_GTIN_SQL,
    GET_PRODUCT_SQL, GET_PROPOSAL_SQL, INSERT_ITEM_SQL, INSERT_PRODUCT_SQL, INSERT_PROPOSAL_SQL,
    INSERT_VERSION_SQL, LIST_ITEMS_SQL, LIST_PROPOSALS_SQL, MAX_FUEL_PAGE,
    PRODUCT_IDENTITY_BY_GTIN_SQL, PRODUCT_IDENTITY_BY_ID_SQL, PRODUCT_IDENTITY_BY_KEY_SQL,
    PROPOSAL_LINK_SQL, PROPOSAL_STATUS_SQL, RECORD_APPROVAL_SQL, REJECT_PROPOSAL_SQL,
    RELINK_PENDING_ITEMS_SQL, REMOVE_ITEM_SQL, RETIRE_DUPLICATE_PENDING_ITEMS_SQL,
    SEARCH_PRODUCTS_SQL, SET_PRODUCT_GTIN_SQL, SUPERSEDE_CURRENT_VERSION_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_fuel_product_repositories!(PostgresDatabase, PgRow, NativeUuid, "BEGIN");
