// ABOUTME: The fuel-product bank's repository traits: the global catalogue and review queue, and what each athlete carries
// ABOUTME: Implemented for both backends by the macro in repositories/fuel_products.rs (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::{
    AthleteFuelItem, CatalogueFuelListing, CatalogueFuelProduct, FuelApprovalOutcome,
    FuelFactOverrides, FuelIngestOutcome, FuelItemCarried, FuelKeyHolder, FuelProductProposal,
    FuelProposalImageMeta, FuelReadRecorded, FuelRejectReason, NewFuelProposal,
    ProposalSourceImage, ProposalStatus, TenantId,
};
use uuid::Uuid;

/// The global fuel-product catalogue and its review queue. Not tenant-scoped
/// by design.
#[async_trait]
pub trait FuelCatalogueRepository: Send + Sync {
    /// Catalogue products whose brand, name and variant contain every word of
    /// `query` (case-folded), or whose GTIN is the barcode `query` names, with
    /// their current facts, at most `limit` of them.
    async fn search_fuel_products(
        &self,
        query: &str,
        limit: i64,
    ) -> AppResult<Vec<CatalogueFuelProduct>>;

    /// One catalogue product with its current facts.
    async fn get_fuel_product(&self, product_id: Uuid) -> AppResult<Option<CatalogueFuelProduct>>;

    /// The product a GTIN-14 names, with its current facts.
    async fn get_fuel_product_by_gtin(&self, gtin: &str)
        -> AppResult<Option<CatalogueFuelProduct>>;

    /// A page of the live catalogue — `limit` (clamped to 1..=100) products
    /// from `offset`, in brand/name/variant order — each with its current
    /// facts and how many versions it has had.
    async fn list_fuel_catalogue(
        &self,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<CatalogueFuelListing>>;

    /// The products `product_ids` name, with their current facts, in the
    /// order asked; an id the catalogue does not hold is left out.
    ///
    /// # Errors
    /// An invalid-input error for more than
    /// [`pierre_core::models::MAX_FUEL_PRODUCT_LOOKUP`] ids.
    async fn get_fuel_products_current(
        &self,
        product_ids: &[Uuid],
    ) -> AppResult<Vec<CatalogueFuelProduct>>;

    /// Queue a proposal for review, with its photos in the order given, in
    /// one transaction, and return its id.
    ///
    /// # Errors
    /// An invalid-input error when its GTIN is not a normalized GTIN-14, or
    /// a database error when its source hash is already queued.
    async fn create_fuel_proposal(&self, proposal: &NewFuelProposal) -> AppResult<Uuid>;

    /// Record a label read the ingestion job paid for, under its
    /// [`pierre_core::models::fuel_read_hash`], so no later run pays for it
    /// again. A [`FuelIngestOutcome::Proposed`] read files `proposal` —
    /// queued for review with its photos and the storefront `listing_key` it
    /// was read from — in the same transaction, and its id comes back; every
    /// other outcome files nothing. A [`FuelIngestOutcome::Duplicate`] read
    /// names the proposal `held_by` that holds its product, and
    /// [`Self::reject_fuel_proposal`] takes it off file. A hash already on
    /// file (a run racing another) is left as it is and nothing is filed:
    /// [`FuelReadRecorded::AlreadyOnFile`].
    ///
    /// # Errors
    /// An invalid-input error when a proposal comes with any outcome but
    /// `Proposed` or none with it, or a holder with any outcome but
    /// `Duplicate` or none with it, or as [`Self::create_fuel_proposal`].
    async fn record_fuel_ingest_read(
        &self,
        source_hash: &str,
        outcome: FuelIngestOutcome,
        proposal: Option<(&NewFuelProposal, &str)>,
        held_by: Option<Uuid>,
    ) -> AppResult<FuelReadRecorded>;

    /// Whether a label read with this hash is on file, whatever it came to.
    async fn fuel_ingest_read_seen(&self, source_hash: &str) -> AppResult<bool>;

    /// What already holds this storefront listing key: the oldest product
    /// entered from it, else the oldest job proposal pending or approved
    /// filed from it — an approved one as the product its approval landed on.
    async fn fuel_listing_holder(&self, listing_key: &str) -> AppResult<Option<FuelKeyHolder>>;

    /// The oldest pending proposal that updates catalogue product
    /// `product_id`, if any.
    async fn pending_fuel_update(&self, product_id: Uuid) -> AppResult<Option<Uuid>>;

    /// What already holds this brand, name and variant (compared by
    /// [`pierre_core::models::fuel_natural_key`]): a catalogue product, else
    /// the oldest proposal pending or approved — an approved one as the
    /// product its approval landed on. A rejected proposal holds
    /// nothing, nor does an athlete's typed filing: a photo-backed read is
    /// better evidence, and the reviewer sees both.
    async fn fuel_key_holder(
        &self,
        brand: &str,
        name: &str,
        variant: &str,
    ) -> AppResult<Option<FuelKeyHolder>>;

    /// Proposals in `status`, oldest first, without their photos.
    async fn list_fuel_proposals(
        &self,
        status: ProposalStatus,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<FuelProductProposal>>;

    /// How many proposals are in `status`: the length of the queue a page
    /// of [`Self::list_fuel_proposals`] is taken from.
    async fn count_fuel_proposals(&self, status: ProposalStatus) -> AppResult<i64>;

    /// How many products the live catalogue lists: the length of the list a
    /// page of [`Self::list_fuel_catalogue`] is taken from.
    async fn count_fuel_catalogue(&self) -> AppResult<i64>;

    /// One proposal, without its photos.
    async fn get_fuel_proposal(&self, proposal_id: Uuid) -> AppResult<Option<FuelProductProposal>>;

    /// The photos stored with a proposal, without their bytes, in the order
    /// they were sent; empty for an unknown proposal or one with none.
    async fn list_fuel_proposal_images(
        &self,
        proposal_id: Uuid,
    ) -> AppResult<Vec<FuelProposalImageMeta>>;

    /// One photo of a proposal, by its position in the order sent (from 0).
    async fn get_fuel_proposal_image(
        &self,
        proposal_id: Uuid,
        position: i64,
    ) -> AppResult<Option<ProposalSourceImage>>;

    /// Approve a pending proposal with an operator's `overrides` (see
    /// [`FuelFactOverrides::review`]): each identity field or fact they set
    /// to a new value or clear is recorded with provenance `admin_edit`,
    /// naming `reviewer_user_id`. The merged facts must pass the plausibility
    /// gate for the reviewed form. In one transaction the proposal's product
    /// is found (its `product_id`, else its reviewed brand, name and variant,
    /// else its reviewed GTIN) or created under the reviewed identity, the
    /// product's current version is superseded, the new one written, the
    /// proposal marked approved under the reviewed identity, and every
    /// athlete item that carried the proposal relinked to the product.
    ///
    /// The operator's identity edits are authoritative: on an existing
    /// product, each brand, name or variant part they changed, the form, and
    /// a GTIN they set or cleared are written onto the product, so the
    /// `admin_edit` provenance never names a correction the product does not
    /// show. An edit whose new key or GTIN another product already holds, or
    /// a GTIN edit (or a key edit) that would land a new read on a barcoded
    /// product filed under another key, is an
    /// [`FuelApprovalOutcome::IdentityConflict`] and writes nothing, as is
    /// one that a concurrent approval beats to that key or barcode. The
    /// proposal's storefront listing key is given to a product that has none.
    ///
    /// # Errors
    /// An invalid-input error for an edit the review refuses, or a database
    /// error.
    async fn approve_fuel_proposal(
        &self,
        proposal_id: Uuid,
        reviewer_user_id: Uuid,
        overrides: &FuelFactOverrides,
    ) -> AppResult<FuelApprovalOutcome>;

    /// Reject a pending proposal, and take off file the `duplicate` label
    /// reads it held, so their listings are read again. Returns whether it
    /// was pending.
    async fn reject_fuel_proposal(
        &self,
        proposal_id: Uuid,
        reviewer_user_id: Uuid,
        reason: FuelRejectReason,
        note: Option<&str>,
    ) -> AppResult<bool>;
}

/// What an athlete carries, in one tenant. Every statement filters on
/// `tenant_id` and `user_id`.
#[async_trait]
pub trait AthleteFuelItemRepository: Send + Sync {
    /// Carry a catalogue product. An athlete who already carries it gets
    /// [`FuelItemCarried::AlreadyCarried`] with that item's id, including
    /// when a concurrent add or an approval's relink wrote it first.
    ///
    /// # Errors
    /// A not-found error when the catalogue holds no such product, or an
    /// invalid-input error at [`pierre_core::models::MAX_FUEL_ITEMS_PER_ATHLETE`].
    async fn add_fuel_product_item(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        product_id: Uuid,
        note: Option<&str>,
    ) -> AppResult<FuelItemCarried>;

    /// Carry a proposal while it is under review. An approved proposal is
    /// carried as its product; an athlete who already carries it gets
    /// [`FuelItemCarried::AlreadyCarried`].
    ///
    /// # Errors
    /// A not-found error for no such proposal, an invalid-input error for a
    /// rejected one or at the item cap.
    async fn add_pending_fuel_item(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        proposal_id: Uuid,
        note: Option<&str>,
    ) -> AppResult<FuelItemCarried>;

    /// File a product the catalogue does not hold for review and carry it
    /// pending, in one transaction under the athlete's item lock: at the
    /// item cap nothing is written, so no proposal is left queued that no
    /// athlete carries. A product the athlete already carries pending review
    /// (same brand, name and variant key, same form) is not filed again: its
    /// item comes back as [`FuelItemCarried::AlreadyCarried`], decided under
    /// the same lock, so two concurrent filings queue one proposal.
    ///
    /// # Errors
    /// An invalid-input error for a GTIN that is not a normalized GTIN-14 or
    /// at [`pierre_core::models::MAX_FUEL_ITEMS_PER_ATHLETE`], or a database
    /// error when its source hash is already queued.
    async fn file_and_carry_fuel_proposal(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        proposal: &NewFuelProposal,
        note: Option<&str>,
    ) -> AppResult<FuelItemCarried>;

    /// The items the athlete carries, oldest first, each resolved to its
    /// product's current facts or to its proposal.
    async fn list_fuel_items(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> AppResult<Vec<AthleteFuelItem>>;

    /// Stop carrying an item. Returns whether the athlete carried it.
    async fn remove_fuel_item(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        item_id: Uuid,
    ) -> AppResult<bool>;
}
