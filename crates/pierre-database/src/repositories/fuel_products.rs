// ABOUTME: Statements and shared body for the fuel-product bank: the global catalogue and review queue, and what each athlete carries
// ABOUTME: The catalogue is not tenant-scoped by design; athlete items are keyed by tenant_id and user_id. Emitted per backend by macro (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The fuel-product bank's storage (carnet#155).
//!
//! `fuel_products`, `fuel_product_versions` and `fuel_product_proposals` are
//! global reference data: the same catalogue for every tenant, so they are
//! not tenant-scoped by design — the coding-standards exception the weather
//! cache and the mobility catalogue use. None of them carries a `user_id`
//! column: the operator and proposer columns are named for their role and set
//! NULL when that account goes.
//!
//! `athlete_fuel_items` is the athlete's own list, and every statement of
//! [`AthleteFuelItemRepository`] filters on `tenant_id` and `user_id`. The one
//! statement that reaches across tenants is the approval's relink
//! ([`RELINK_PENDING_ITEMS_SQL`]): a proposal is global, so approving it hands
//! its product to every athlete who carries it while it was pending, keyed by
//! the proposal and never by an athlete.
//!
//! Facts reach the catalogue only through [`FuelCatalogueRepository::approve_proposal`],
//! which passes the merged facts through
//! [`pierre_core::models::check_plausibility`] and, in one transaction,
//! supersedes the product's current version and writes the new one.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres, so one statement serves both backends. Ids bind through the
//! backend's uuid codec (see [`super::uuid_columns`]); timestamps bind as
//! `DateTime<Utc>`, facts as `Option<f64>`, photos as bytes, and each driver
//! encodes them for its own column type.

mod rows;
mod sql;
mod traits;

pub(crate) use rows::{
    bind_fuel_facts, catalogue_from_row, check_gtin, from_json, fuel_error, image_meta_from_row,
    merge_for_approval, parse_form, parse_status, proposal_from_row, search_patterns, to_json,
    CatalogueIds, FuelIdentityKey, ProductIdentity, ProposalIds, MAX_FUEL_PAGE,
};
pub(crate) use sql::{
    CLAIM_PROPOSAL_SQL, COUNT_INGEST_READS_SQL, COUNT_LIVE_ITEMS_SQL, COUNT_PRODUCT_SQL,
    FIND_LIVE_PENDING_FILING_SQL, FIND_LIVE_PENDING_ITEM_SQL, FIND_LIVE_PRODUCT_ITEM_SQL,
    GET_PRODUCT_BY_GTIN_SQL, GET_PRODUCT_SQL, GET_PROPOSAL_IMAGE_SQL, GET_PROPOSAL_SQL,
    HOLD_PENDING_PROPOSAL_SQL, INSERT_ITEM_SQL, INSERT_PRODUCT_SQL, INSERT_PROPOSAL_IMAGE_SQL,
    INSERT_PROPOSAL_SQL, INSERT_VERSION_SQL, LIST_CATALOGUE_SQL, LIST_ITEMS_SQL,
    LIST_PROPOSALS_SQL, LIST_PROPOSAL_IMAGES_SQL, PRODUCT_IDENTITY_BY_GTIN_SQL,
    PRODUCT_IDENTITY_BY_ID_SQL, PRODUCT_IDENTITY_BY_KEY_SQL, PROPOSAL_HOLDING_KEY_SQL,
    PROPOSAL_LINK_SQL, PROPOSAL_STATUS_SQL, RECORD_APPROVAL_SQL, REJECT_PROPOSAL_SQL,
    RELINK_PENDING_ITEMS_SQL, REMOVE_ITEM_SQL, RETIRE_DUPLICATE_PENDING_ITEMS_SQL,
    SEARCH_PRODUCTS_SQL, SET_PRODUCT_GTIN_SQL, SUPERSEDE_CURRENT_VERSION_SQL,
};
pub(crate) use sql::{
    COUNT_CATALOGUE_SQL, COUNT_PROPOSALS_BY_STATUS_SQL, PENDING_UPDATE_SQL,
    PRODUCT_HOLDING_LISTING_SQL, PROPOSAL_HOLDING_LISTING_SQL, RECORD_INGEST_READ_SQL,
    REPLACE_PRODUCT_GTIN_SQL, SET_PRODUCT_FORM_SQL, SET_PRODUCT_IDENTITY_SQL,
    SET_PRODUCT_LISTING_KEY_SQL,
};
pub use traits::{AthleteFuelItemRepository, FuelCatalogueRepository};

/// Emit both fuel-product repository implementations for one backend type.
/// `$row` is that backend's row type, `$ids` its uuid codec
/// ([`crate::repositories::uuid_columns`]) and `$begin` the statement that
/// opens a write transaction (`BEGIN IMMEDIATE` on `SQLite`, so the write lock
/// is held before the approval reads). `$owner_lock` is the statement, if the
/// backend needs one, that serializes one athlete's item writes inside that
/// transaction: `SQLite`'s `BEGIN IMMEDIATE` already does, Postgres takes a
/// transaction-scoped advisory lock keyed by the athlete.
macro_rules! impl_fuel_product_repositories {
    ($ty:ty, $row:ty, $ids:ident, $begin:literal, $owner_lock:expr) => {
        impl $ty {
            fn fuel_catalogue_row(row: &$row) -> AppResult<CatalogueFuelProduct> {
                catalogue_from_row(
                    row,
                    CatalogueIds {
                        product: $ids::read(row, "p_id")?,
                        version: $ids::read(row, "v_id")?,
                        superseded_by: $ids::read_opt(row, "superseded_by")?,
                        approved_by: $ids::read_opt(row, "approved_by_user_id")?,
                    },
                )
            }

            fn fuel_proposal_row(row: &$row) -> AppResult<FuelProductProposal> {
                proposal_from_row(
                    row,
                    ProposalIds {
                        id: $ids::read(row, "id")?,
                        product: $ids::read_opt(row, "product_id")?,
                        proposed_by: $ids::read_opt(row, "proposed_by_user_id")?,
                        reviewed_by: $ids::read_opt(row, "reviewed_by_user_id")?,
                        approved_version: $ids::read_opt(row, "approved_version_id")?,
                    },
                )
            }

            /// What a proposal holding a key or a listing amounts to: an
            /// approved one is the product its approval landed on, a pending
            /// one itself.
            fn fuel_holder_row(row: &$row) -> AppResult<FuelKeyHolder> {
                use sqlx::Row as _;
                let status: String = row
                    .try_get("status")
                    .map_err(|e| fuel_error("decode status of", e))?;
                match (parse_status(&status)?, $ids::read_opt(row, "product_id")?) {
                    (ProposalStatus::Approved, Some(product)) => Ok(FuelKeyHolder::Product(product)),
                    _ => Ok(FuelKeyHolder::Proposal($ids::read(row, "id")?)),
                }
            }

            fn fuel_item_row(row: &$row) -> AppResult<AthleteFuelItem> {
                use sqlx::Row as _;
                let column = |col: &str, e: sqlx::Error| fuel_error(&format!("decode {col} of"), e);
                let entry = if $ids::read_opt(row, "i_product_id")?.is_some() {
                    FuelItemEntry::Catalogue(Self::fuel_catalogue_row(row)?)
                } else {
                    let proposal_id = $ids::read(row, "i_proposal_id")?;
                    let form: String = row.try_get("pr_form").map_err(|e| column("pr_form", e))?;
                    let facts: String = row
                        .try_get("pr_facts_json")
                        .map_err(|e| column("pr_facts_json", e))?;
                    let status: String =
                        row.try_get("pr_status").map_err(|e| column("pr_status", e))?;
                    FuelItemEntry::Pending(PendingFuelItem {
                        proposal_id,
                        brand: row.try_get("pr_brand").map_err(|e| column("pr_brand", e))?,
                        name: row.try_get("pr_name").map_err(|e| column("pr_name", e))?,
                        variant: row
                            .try_get("pr_variant")
                            .map_err(|e| column("pr_variant", e))?,
                        form: parse_form(&form)?,
                        facts: from_json("facts", &facts)?,
                        status: parse_status(&status)?,
                    })
                };
                Ok(AthleteFuelItem {
                    id: $ids::read(row, "i_id")?,
                    note: row.try_get("i_note").map_err(|e| column("i_note", e))?,
                    created_at: row
                        .try_get("i_created_at")
                        .map_err(|e| column("i_created_at", e))?,
                    entry,
                })
            }

            /// The id, GTIN and natural key of the product a statement
            /// selects, if any.
            async fn fuel_product_identity(
                tx: &mut sqlx::Transaction<'static, <$row as sqlx::Row>::Database>,
                sql: &'static str,
                key: FuelIdentityKey<'_>,
            ) -> AppResult<Option<$crate::repositories::fuel_products::ProductIdentity>> {
                use sqlx::Row as _;
                let query = sqlx::query(sql);
                let query = match key {
                    FuelIdentityKey::Id(id) => query.bind($ids::bind(id)),
                    FuelIdentityKey::Text(text) => query.bind(text.to_owned()),
                };
                let row = query
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|e| fuel_error("look up a product in", e))?;
                row.map(|row| {
                    Ok($crate::repositories::fuel_products::ProductIdentity {
                        id: $ids::read(&row, "id")?,
                        gtin: row.try_get("gtin").map_err(|e| fuel_error("decode gtin of", e))?,
                        brand: row.try_get("brand").map_err(|e| fuel_error("decode brand of", e))?,
                        name: row.try_get("name").map_err(|e| fuel_error("decode name of", e))?,
                        variant: row
                            .try_get("variant")
                            .map_err(|e| fuel_error("decode variant of", e))?,
                        natural_key: row
                            .try_get("natural_key")
                            .map_err(|e| fuel_error("decode natural_key of", e))?,
                    })
                })
                .transpose()
            }

            /// The product an approval lands on, or `Err` with the one it
            /// conflicts with.
            ///
            /// The product the proposal updates (`product_id`), else the one
            /// its reviewed brand, name and variant name; else the one its
            /// reviewed GTIN names; else a new one under the reviewed
            /// identity, given `listing_key`. A conflict is a barcode another
            /// product holds, a named product holding another barcode than
            /// the one read, or an operator's key or GTIN edit that would land
            /// a new read on a barcoded product filed under another key. On an
            /// existing product the operator's edits are then applied
            /// ([`Self::apply_fuel_identity_edits`]).
            async fn resolve_fuel_product(
                tx: &mut sqlx::Transaction<'static, <$row as sqlx::Row>::Database>,
                reviewed: &pierre_core::models::ReviewedProposal,
                product_id: Option<Uuid>,
                listing_key: Option<&str>,
                now: DateTime<Utc>,
            ) -> AppResult<Result<Uuid, Uuid>> {
                let identity = &reviewed.identity;
                let key = identity.natural_key();
                let key_edited =
                    reviewed.changed("brand") || reviewed.changed("name") || reviewed.changed("variant");
                let gtin_edited = reviewed.changed("gtin");
                let by_gtin = match identity.gtin.as_deref() {
                    Some(gtin) => {
                        Self::fuel_product_identity(
                            tx,
                            PRODUCT_IDENTITY_BY_GTIN_SQL,
                            FuelIdentityKey::Text(gtin),
                        )
                        .await?
                    }
                    None => None,
                };
                let named = match product_id {
                    Some(id) => {
                        Self::fuel_product_identity(
                            tx,
                            PRODUCT_IDENTITY_BY_ID_SQL,
                            FuelIdentityKey::Id(id),
                        )
                        .await?
                    }
                    None => {
                        Self::fuel_product_identity(
                            tx,
                            PRODUCT_IDENTITY_BY_KEY_SQL,
                            FuelIdentityKey::Text(&key),
                        )
                        .await?
                    }
                };
                let existing = match (named, by_gtin) {
                    (Some(named), Some(holder)) if holder.id != named.id => return Ok(Err(holder.id)),
                    // The named product carries another barcode than the one
                    // read; an operator's barcode replaces it.
                    (Some(named), None)
                        if named.gtin.is_some() && identity.gtin.is_some() && !gtin_edited =>
                    {
                        return Ok(Err(named.id))
                    }
                    (Some(named), _) => named,
                    (None, Some(holder)) => {
                        // Nothing holds the reviewed key, so the barcode's
                        // product is filed under another one: a read may land
                        // on it by its printed barcode, an operator's edit
                        // never moves a read onto it.
                        if key_edited || gtin_edited {
                            return Ok(Err(holder.id));
                        }
                        holder
                    }
                    (None, None) => {
                        let id = Uuid::new_v4();
                        sqlx::query(INSERT_PRODUCT_SQL)
                            .bind($ids::bind(id))
                            .bind(identity.gtin.as_deref())
                            .bind(identity.brand.as_str())
                            .bind(identity.name.as_str())
                            .bind(identity.variant.as_str())
                            .bind(identity.form.as_str())
                            .bind(key.as_str())
                            .bind(now)
                            .bind(listing_key)
                            .execute(&mut **tx)
                            .await
                            .map_err(|e| fuel_error("enter a product in", e))?;
                        return Ok(Ok(id));
                    }
                };
                if let Err(other) = Self::apply_fuel_identity_edits(tx, reviewed, &existing).await? {
                    return Ok(Err(other));
                }
                if let Some(listing_key) = listing_key {
                    sqlx::query($crate::repositories::fuel_products::SET_PRODUCT_LISTING_KEY_SQL)
                        .bind($ids::bind(existing.id))
                        .bind(listing_key)
                        .execute(&mut **tx)
                        .await
                        .map_err(|e| fuel_error("set a product's listing in", e))?;
                }
                Ok(Ok(existing.id))
            }

            /// Write onto `existing` what the review changed: each brand,
            /// name or variant part the operator spelled (case corrections
            /// included), the form, and a GTIN they set or cleared. A barcode
            /// read and never edited only fills a product that has none.
            /// `Err` with the product that already holds the renamed key,
            /// checked before any write so the unique index never fires.
            async fn apply_fuel_identity_edits(
                tx: &mut sqlx::Transaction<'static, <$row as sqlx::Row>::Database>,
                reviewed: &pierre_core::models::ReviewedProposal,
                existing: &$crate::repositories::fuel_products::ProductIdentity,
            ) -> AppResult<Result<(), Uuid>> {
                let identity = &reviewed.identity;
                if reviewed.changed("brand") || reviewed.changed("name") || reviewed.changed("variant") {
                    let (brand, name, variant) = existing.renamed_by(reviewed);
                    let key = fuel_natural_key(&brand, &name, &variant);
                    if key != existing.natural_key {
                        let holder = Self::fuel_product_identity(
                            tx,
                            PRODUCT_IDENTITY_BY_KEY_SQL,
                            FuelIdentityKey::Text(&key),
                        )
                        .await?;
                        if let Some(holder) = holder.filter(|holder| holder.id != existing.id) {
                            return Ok(Err(holder.id));
                        }
                    }
                    sqlx::query($crate::repositories::fuel_products::SET_PRODUCT_IDENTITY_SQL)
                        .bind($ids::bind(existing.id))
                        .bind(brand)
                        .bind(name)
                        .bind(variant)
                        .bind(key)
                        .execute(&mut **tx)
                        .await
                        .map_err(|e| fuel_error("correct a product's identity in", e))?;
                }
                if reviewed.changed("gtin") {
                    sqlx::query($crate::repositories::fuel_products::REPLACE_PRODUCT_GTIN_SQL)
                        .bind($ids::bind(existing.id))
                        .bind(identity.gtin.as_deref())
                        .execute(&mut **tx)
                        .await
                        .map_err(|e| fuel_error("correct a product's GTIN in", e))?;
                } else if existing.gtin.is_none() {
                    if let Some(gtin) = identity.gtin.as_deref() {
                        sqlx::query(SET_PRODUCT_GTIN_SQL)
                            .bind($ids::bind(existing.id))
                            .bind(gtin)
                            .execute(&mut **tx)
                            .await
                            .map_err(|e| fuel_error("set a product's GTIN in", e))?;
                    }
                }
                if reviewed.changed("form") {
                    sqlx::query($crate::repositories::fuel_products::SET_PRODUCT_FORM_SQL)
                        .bind($ids::bind(existing.id))
                        .bind(identity.form.as_str())
                        .execute(&mut **tx)
                        .await
                        .map_err(|e| fuel_error("correct a product's form in", e))?;
                }
                Ok(Ok(()))
            }

            /// Supersede `product_id`'s current version; returns the new version's id.
            async fn write_fuel_version(
                tx: &mut sqlx::Transaction<'static, <$row as sqlx::Row>::Database>,
                product_id: Uuid,
                facts: &FuelFacts,
                provenance: &FactProvenance,
                reviewer_user_id: Uuid,
                now: DateTime<Utc>,
            ) -> AppResult<Uuid> {
                let version_id = Uuid::new_v4();
                sqlx::query(SUPERSEDE_CURRENT_VERSION_SQL)
                    .bind($ids::bind(product_id))
                    .bind($ids::bind(version_id))
                    .execute(&mut **tx)
                    .await
                    .map_err(|e| fuel_error("supersede a version in", e))?;
                let query = sqlx::query(INSERT_VERSION_SQL)
                    .bind($ids::bind(version_id))
                    .bind($ids::bind(product_id))
                    .bind(now);
                $crate::repositories::fuel_products::bind_fuel_facts!(query, facts)
                    .bind(to_json("provenance", provenance)?)
                    .bind($ids::bind(reviewer_user_id))
                    .bind(now)
                    .execute(&mut **tx)
                    .await
                    .map_err(|e| fuel_error("write a version to", e))?;
                Ok(version_id)
            }

            /// Hand `product_id` to the items that carried `proposal_id`.
            async fn relink_fuel_items(
                tx: &mut sqlx::Transaction<'static, <$row as sqlx::Row>::Database>,
                proposal_id: Uuid,
                product_id: Uuid,
                now: DateTime<Utc>,
            ) -> AppResult<()> {
                sqlx::query(RELINK_PENDING_ITEMS_SQL)
                    .bind($ids::bind(proposal_id))
                    .bind($ids::bind(product_id))
                    .execute(&mut **tx)
                    .await
                    .map_err(|e| fuel_error("relink pending items in", e))?;
                sqlx::query(RETIRE_DUPLICATE_PENDING_ITEMS_SQL)
                    .bind($ids::bind(proposal_id))
                    .bind($ids::bind(product_id))
                    .bind(now)
                    .execute(&mut **tx)
                    .await
                    .map_err(|e| fuel_error("close duplicate pending items in", e))?;
                Ok(())
            }

            /// Open the transaction one athlete's item is written in, holding
            /// that athlete's item writes until it ends, so the cap count, the
            /// duplicate check and the insert see one state under concurrent
            /// adds. `SQLite` holds the write lock from `BEGIN IMMEDIATE`.
            /// Postgres takes the athlete's advisory lock before its first
            /// read: a row lock could not do it, since at READ COMMITTED a
            /// count that waited on locked rows still misses a row inserted
            /// meanwhile, and the next statement's snapshot sees what the
            /// writer before it committed.
            async fn begin_fuel_item_write(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
            ) -> AppResult<sqlx::Transaction<'static, <$row as sqlx::Row>::Database>> {
                let mut tx = self
                    .pool()
                    .begin_with($begin)
                    .await
                    .map_err(|e| fuel_error("begin an item write in", e))?;
                let owner_lock: Option<&'static str> = $owner_lock;
                if let Some(lock) = owner_lock {
                    sqlx::query(lock)
                        .bind(format!("athlete_fuel_items:{tenant_id}:{user_id}"))
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| fuel_error("lock an athlete's items in", e))?;
                }
                Ok(tx)
            }

            /// The live item a unique-index collision met: what the athlete
            /// carries, read after the failed transaction is dropped (Postgres
            /// aborts it on the error).
            async fn colliding_fuel_item(
                &self,
                sql: &'static str,
                tenant_id: TenantId,
                user_id: Uuid,
                key: Uuid,
            ) -> AppResult<FuelItemCarried> {
                let row = sqlx::query(sql)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(key))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| fuel_error("find an item in", e))?;
                match row {
                    Some(row) => Ok(FuelItemCarried::AlreadyCarried($ids::read(&row, "id")?)),
                    None => Err(AppError::database(
                        "A fuel item collided with one that is no longer carried; add it again",
                    )),
                }
            }

            /// Insert an item unless the athlete is at the cap. `Ok(None)`
            /// when a live item for the same product or proposal holds the
            /// unique index: one written outside this athlete's lock, by an
            /// approval relinking a pending item.
            async fn insert_fuel_item(
                tx: &mut sqlx::Transaction<'static, <$row as sqlx::Row>::Database>,
                tenant_id: TenantId,
                user_id: Uuid,
                product_id: Option<Uuid>,
                proposal_id: Option<Uuid>,
                note: Option<&str>,
            ) -> AppResult<Option<Uuid>> {
                let live: i64 = sqlx::query_scalar(COUNT_LIVE_ITEMS_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(|e| fuel_error("count items in", e))?;
                if live >= MAX_FUEL_ITEMS_PER_ATHLETE {
                    return Err(AppError::invalid_input(format!(
                        "An athlete carries at most {MAX_FUEL_ITEMS_PER_ATHLETE} fuel items; remove one first"
                    )));
                }
                let id = Uuid::new_v4();
                let inserted = sqlx::query(INSERT_ITEM_SQL)
                    .bind($ids::bind(id))
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind($ids::bind_opt(product_id))
                    .bind($ids::bind_opt(proposal_id))
                    .bind(note)
                    .bind(Utc::now())
                    .execute(&mut **tx)
                    .await;
                match inserted {
                    Ok(_) => Ok(Some(id)),
                    Err(e)
                        if e
                            .as_database_error()
                            .is_some_and(|db| db.is_unique_violation()) =>
                    {
                        Ok(None)
                    }
                    Err(e) => Err(fuel_error("add an item to", e)),
                }
            }

            /// Queue `proposal` inside `tx`, its photos in the order given,
            /// filed from storefront `listing_key` when a job read filed it,
            /// and return its id. Its GTIN is checked by the caller.
            async fn insert_fuel_proposal(
                tx: &mut sqlx::Transaction<'static, <$row as sqlx::Row>::Database>,
                proposal: &NewFuelProposal,
                listing_key: Option<&str>,
            ) -> AppResult<Uuid> {
                let id = Uuid::new_v4();
                sqlx::query(INSERT_PROPOSAL_SQL)
                    .bind($ids::bind(id))
                    .bind($ids::bind_opt(proposal.product_id))
                    .bind(proposal.brand.as_str())
                    .bind(proposal.name.as_str())
                    .bind(proposal.variant.as_str())
                    .bind(proposal.form.as_str())
                    .bind(fuel_natural_key(&proposal.brand, &proposal.name, &proposal.variant))
                    .bind(proposal.gtin.as_deref())
                    .bind(proposal.source_kind.as_str())
                    .bind(proposal.source_url.as_deref())
                    .bind(proposal.source_hash.as_deref())
                    .bind(to_json("facts", &proposal.facts)?)
                    .bind(to_json("provenance", &proposal.provenance)?)
                    .bind(to_json("gate flags", &proposal.gate_flags)?)
                    .bind(proposal.notes.as_deref())
                    .bind(proposal.ingredients_text.as_deref())
                    .bind(proposal.multiple_transportable_stated)
                    .bind($ids::bind_opt(proposal.proposed_by_user_id))
                    .bind(Utc::now())
                    .bind(listing_key)
                    .execute(&mut **tx)
                    .await
                    .map_err(|e| fuel_error("queue a proposal in", e))?;
                for (position, image) in proposal.images.iter().enumerate() {
                    let position = i32::try_from(position).map_err(|_| {
                        AppError::invalid_input("A fuel proposal carries too many photos")
                    })?;
                    sqlx::query(INSERT_PROPOSAL_IMAGE_SQL)
                        .bind($ids::bind(id))
                        .bind(position)
                        .bind(image.bytes.as_slice())
                        .bind(image.mime.as_str())
                        .bind(image_sha256(&image.bytes))
                        .execute(&mut **tx)
                        .await
                        .map_err(|e| fuel_error("store a proposal photo in", e))?;
                }
                Ok(id)
            }

            /// The live item of the athlete a lookup selects, if any.
            async fn find_live_fuel_item(
                tx: &mut sqlx::Transaction<'static, <$row as sqlx::Row>::Database>,
                sql: &'static str,
                tenant_id: TenantId,
                user_id: Uuid,
                key: Uuid,
            ) -> AppResult<Option<Uuid>> {
                let row = sqlx::query(sql)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(key))
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|e| fuel_error("find an item in", e))?;
                row.map(|row| $ids::read(&row, "id")).transpose()
            }
        }

        #[async_trait::async_trait]
        impl FuelCatalogueRepository for $ty {
            async fn search_fuel_products(
                &self,
                query: &str,
                limit: i64,
            ) -> AppResult<Vec<CatalogueFuelProduct>> {
                let [first, second, third, fourth] = search_patterns(query);
                let rows = sqlx::query(SEARCH_PRODUCTS_SQL)
                    .bind(normalize_gtin(query))
                    .bind(first)
                    .bind(second)
                    .bind(third)
                    .bind(fourth)
                    .bind(limit.clamp(1, MAX_FUEL_PAGE))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| fuel_error("search", e))?;
                rows.iter().map(Self::fuel_catalogue_row).collect()
            }

            async fn get_fuel_product(
                &self,
                product_id: Uuid,
            ) -> AppResult<Option<CatalogueFuelProduct>> {
                let row = sqlx::query(GET_PRODUCT_SQL)
                    .bind($ids::bind(product_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| fuel_error("read", e))?;
                row.map(|row| Self::fuel_catalogue_row(&row)).transpose()
            }

            async fn get_fuel_product_by_gtin(
                &self,
                gtin: &str,
            ) -> AppResult<Option<CatalogueFuelProduct>> {
                let row = sqlx::query(GET_PRODUCT_BY_GTIN_SQL)
                    .bind(gtin)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| fuel_error("read", e))?;
                row.map(|row| Self::fuel_catalogue_row(&row)).transpose()
            }

            async fn list_fuel_catalogue(
                &self,
                limit: i64,
                offset: i64,
            ) -> AppResult<Vec<pierre_core::models::CatalogueFuelListing>> {
                use sqlx::Row as _;
                let rows = sqlx::query($crate::repositories::fuel_products::LIST_CATALOGUE_SQL)
                    .bind(limit.clamp(1, MAX_FUEL_PAGE))
                    .bind(offset.max(0))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| fuel_error("list the catalogue of", e))?;
                rows.iter()
                    .map(|row| {
                        Ok(pierre_core::models::CatalogueFuelListing {
                            product: Self::fuel_catalogue_row(row)?,
                            version_count: row
                                .try_get("version_count")
                                .map_err(|e| fuel_error("decode a version count of", e))?,
                        })
                    })
                    .collect()
            }

            async fn get_fuel_products_current(
                &self,
                product_ids: &[Uuid],
            ) -> AppResult<Vec<CatalogueFuelProduct>> {
                if product_ids.len() > MAX_FUEL_PRODUCT_LOOKUP {
                    return Err(AppError::invalid_input(format!(
                        "At most {MAX_FUEL_PRODUCT_LOOKUP} fuel products are looked up at once"
                    )));
                }
                // One indexed read per id: the set is small and bounded, and a
                // per-id statement is the same SQL on both engines, where an
                // id list is not.
                let mut found = Vec::with_capacity(product_ids.len());
                for id in product_ids {
                    if let Some(product) = self.get_fuel_product(*id).await? {
                        found.push(product);
                    }
                }
                Ok(found)
            }

            async fn create_fuel_proposal(&self, proposal: &NewFuelProposal) -> AppResult<Uuid> {
                check_gtin(proposal.gtin.as_deref())?;
                let mut tx = self
                    .pool()
                    .begin()
                    .await
                    .map_err(|e| fuel_error("begin a proposal in", e))?;
                let id = Self::insert_fuel_proposal(&mut tx, proposal, None).await?;
                tx.commit()
                    .await
                    .map_err(|e| fuel_error("commit a proposal to", e))?;
                Ok(id)
            }

            async fn record_fuel_ingest_read(
                &self,
                source_hash: &str,
                outcome: pierre_core::models::FuelIngestOutcome,
                proposal: Option<(&NewFuelProposal, &str)>,
            ) -> AppResult<Option<Uuid>> {
                let proposed = outcome == pierre_core::models::FuelIngestOutcome::Proposed;
                if proposed != proposal.is_some() {
                    return Err(AppError::invalid_input(format!(
                        "A label read that came to `{}` {} a proposal",
                        outcome.as_str(),
                        if proposed { "must file" } else { "files no" }
                    )));
                }
                if let Some((filed, _)) = proposal {
                    check_gtin(filed.gtin.as_deref())?;
                }
                let mut tx = self
                    .pool()
                    .begin()
                    .await
                    .map_err(|e| fuel_error("begin a label read in", e))?;
                let filed = match proposal {
                    Some((filed, listing_key)) => {
                        Some(Self::insert_fuel_proposal(&mut tx, filed, Some(listing_key)).await?)
                    }
                    None => None,
                };
                sqlx::query($crate::repositories::fuel_products::RECORD_INGEST_READ_SQL)
                    .bind(source_hash)
                    .bind(outcome.as_str())
                    .bind($ids::bind_opt(filed))
                    .bind(Utc::now())
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| fuel_error("record a label read in", e))?;
                tx.commit()
                    .await
                    .map_err(|e| fuel_error("commit a label read to", e))?;
                Ok(filed)
            }

            async fn fuel_ingest_read_seen(&self, source_hash: &str) -> AppResult<bool> {
                let count: i64 = sqlx::query_scalar(COUNT_INGEST_READS_SQL)
                    .bind(source_hash)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| fuel_error("look up a label read in", e))?;
                Ok(count > 0)
            }

            async fn fuel_listing_holder(&self, listing_key: &str) -> AppResult<Option<FuelKeyHolder>> {
                let product = sqlx::query($crate::repositories::fuel_products::PRODUCT_HOLDING_LISTING_SQL)
                    .bind(listing_key)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| fuel_error("look up a listing's product in", e))?;
                if let Some(row) = product {
                    return Ok(Some(FuelKeyHolder::Product($ids::read(&row, "id")?)));
                }
                let proposal = sqlx::query($crate::repositories::fuel_products::PROPOSAL_HOLDING_LISTING_SQL)
                    .bind(listing_key)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| fuel_error("look up a listing's proposal in", e))?;
                proposal.map(|row| Self::fuel_holder_row(&row)).transpose()
            }

            async fn pending_fuel_update(&self, product_id: Uuid) -> AppResult<Option<Uuid>> {
                let row = sqlx::query($crate::repositories::fuel_products::PENDING_UPDATE_SQL)
                    .bind($ids::bind(product_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| fuel_error("look up a pending update in", e))?;
                row.map(|row| $ids::read(&row, "id")).transpose()
            }

            async fn fuel_key_holder(
                &self,
                brand: &str,
                name: &str,
                variant: &str,
            ) -> AppResult<Option<FuelKeyHolder>> {
                let key = fuel_natural_key(brand, name, variant);
                let product = sqlx::query(PRODUCT_IDENTITY_BY_KEY_SQL)
                    .bind(key.as_str())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| fuel_error("look up a product in", e))?;
                if let Some(row) = product {
                    return Ok(Some(FuelKeyHolder::Product($ids::read(&row, "id")?)));
                }
                let proposal = sqlx::query(PROPOSAL_HOLDING_KEY_SQL)
                    .bind(key.as_str())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| fuel_error("look up a proposal in", e))?;
                proposal.map(|row| Self::fuel_holder_row(&row)).transpose()
            }

            async fn list_fuel_proposals(
                &self,
                status: ProposalStatus,
                limit: i64,
                offset: i64,
            ) -> AppResult<Vec<FuelProductProposal>> {
                let rows = sqlx::query(LIST_PROPOSALS_SQL)
                    .bind(status.as_str())
                    .bind(limit.clamp(1, MAX_FUEL_PAGE))
                    .bind(offset.max(0))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| fuel_error("list proposals in", e))?;
                rows.iter().map(Self::fuel_proposal_row).collect()
            }

            async fn count_fuel_proposals(&self, status: ProposalStatus) -> AppResult<i64> {
                sqlx::query_scalar($crate::repositories::fuel_products::COUNT_PROPOSALS_BY_STATUS_SQL)
                    .bind(status.as_str())
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| fuel_error("count proposals in", e))
            }

            async fn count_fuel_catalogue(&self) -> AppResult<i64> {
                sqlx::query_scalar($crate::repositories::fuel_products::COUNT_CATALOGUE_SQL)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| fuel_error("count the catalogue of", e))
            }

            async fn get_fuel_proposal(
                &self,
                proposal_id: Uuid,
            ) -> AppResult<Option<FuelProductProposal>> {
                let row = sqlx::query(GET_PROPOSAL_SQL)
                    .bind($ids::bind(proposal_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| fuel_error("read a proposal in", e))?;
                row.map(|row| Self::fuel_proposal_row(&row)).transpose()
            }

            async fn list_fuel_proposal_images(
                &self,
                proposal_id: Uuid,
            ) -> AppResult<Vec<FuelProposalImageMeta>> {
                let rows = sqlx::query(LIST_PROPOSAL_IMAGES_SQL)
                    .bind($ids::bind(proposal_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| fuel_error("list a proposal's photos in", e))?;
                rows.iter().map(image_meta_from_row).collect()
            }

            async fn get_fuel_proposal_image(
                &self,
                proposal_id: Uuid,
                position: i64,
            ) -> AppResult<Option<ProposalSourceImage>> {
                use sqlx::Row as _;
                let Ok(position) = i32::try_from(position) else {
                    return Ok(None);
                };
                let row = sqlx::query(GET_PROPOSAL_IMAGE_SQL)
                    .bind($ids::bind(proposal_id))
                    .bind(position)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| fuel_error("read a proposal photo in", e))?;
                row.map(|row| {
                    Ok(ProposalSourceImage {
                        bytes: row
                            .try_get("bytes")
                            .map_err(|e| fuel_error("decode bytes of", e))?,
                        mime: row
                            .try_get("mime")
                            .map_err(|e| fuel_error("decode mime of", e))?,
                    })
                })
                .transpose()
            }

            async fn approve_fuel_proposal(
                &self,
                proposal_id: Uuid,
                reviewer_user_id: Uuid,
                overrides: &pierre_core::models::FuelFactOverrides,
            ) -> AppResult<FuelApprovalOutcome> {
                let Some(proposal) = self.get_fuel_proposal(proposal_id).await? else {
                    return Ok(FuelApprovalOutcome::NotFound);
                };
                if proposal.status != ProposalStatus::Pending {
                    return Ok(FuelApprovalOutcome::NotPending(proposal.status));
                }
                let (reviewed, provenance) =
                    merge_for_approval(&proposal, overrides, reviewer_user_id)?;
                if let Err(violations) =
                    check_plausibility(reviewed.identity.form, &reviewed.facts)
                {
                    return Ok(FuelApprovalOutcome::Implausible(violations));
                }
                let now = Utc::now();
                let mut tx = self
                    .pool()
                    .begin_with($begin)
                    .await
                    .map_err(|e| fuel_error("begin an approval in", e))?;
                let claimed = sqlx::query(CLAIM_PROPOSAL_SQL)
                    .bind($ids::bind(proposal_id))
                    .bind($ids::bind(reviewer_user_id))
                    .bind(now)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| fuel_error("claim a proposal in", e))?
                    .rows_affected();
                if claimed != 1 {
                    let status: String = sqlx::query_scalar(PROPOSAL_STATUS_SQL)
                        .bind($ids::bind(proposal_id))
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(|e| fuel_error("read a proposal in", e))?;
                    return Ok(FuelApprovalOutcome::NotPending(parse_status(&status)?));
                }
                let product_id = match Self::resolve_fuel_product(
                    &mut tx,
                    &reviewed,
                    proposal.product_id,
                    proposal.listing_key.as_deref(),
                    now,
                )
                .await?
                {
                    Ok(id) => id,
                    // Dropping the transaction rolls the claim back.
                    Err(product_id) => {
                        return Ok(FuelApprovalOutcome::IdentityConflict { product_id })
                    }
                };
                let version_id = Self::write_fuel_version(
                    &mut tx,
                    product_id,
                    &reviewed.facts,
                    &provenance,
                    reviewer_user_id,
                    now,
                )
                .await?;
                sqlx::query(RECORD_APPROVAL_SQL)
                    .bind($ids::bind(proposal_id))
                    .bind($ids::bind(version_id))
                    .bind($ids::bind(product_id))
                    .bind(reviewed.identity.brand.as_str())
                    .bind(reviewed.identity.name.as_str())
                    .bind(reviewed.identity.variant.as_str())
                    .bind(reviewed.identity.form.as_str())
                    .bind(reviewed.identity.natural_key())
                    .bind(reviewed.identity.gtin.as_deref())
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| fuel_error("record an approval in", e))?;
                Self::relink_fuel_items(&mut tx, proposal_id, product_id, now).await?;
                tx.commit()
                    .await
                    .map_err(|e| fuel_error("commit an approval to", e))?;
                let approved = self.get_fuel_product(product_id).await?.ok_or_else(|| {
                    AppError::database("An approved fuel product has no current version")
                })?;
                Ok(FuelApprovalOutcome::Approved(Box::new(approved)))
            }

            async fn reject_fuel_proposal(
                &self,
                proposal_id: Uuid,
                reviewer_user_id: Uuid,
                reason: FuelRejectReason,
                note: Option<&str>,
            ) -> AppResult<bool> {
                let rejected = sqlx::query(REJECT_PROPOSAL_SQL)
                    .bind($ids::bind(proposal_id))
                    .bind($ids::bind(reviewer_user_id))
                    .bind(Utc::now())
                    .bind(reason.as_str())
                    .bind(note)
                    .execute(self.pool())
                    .await
                    .map_err(|e| fuel_error("reject a proposal in", e))?
                    .rows_affected();
                Ok(rejected == 1)
            }
        }

        #[async_trait::async_trait]
        impl AthleteFuelItemRepository for $ty {
            async fn add_fuel_product_item(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                product_id: Uuid,
                note: Option<&str>,
            ) -> AppResult<FuelItemCarried> {
                let mut tx = self.begin_fuel_item_write(tenant_id, user_id).await?;
                let exists: i64 = sqlx::query_scalar(COUNT_PRODUCT_SQL)
                    .bind($ids::bind(product_id))
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(|e| fuel_error("look up a product in", e))?;
                if exists == 0 {
                    return Err(AppError::not_found(format!("Fuel product {product_id}")));
                }
                if let Some(held) = Self::find_live_fuel_item(
                    &mut tx,
                    FIND_LIVE_PRODUCT_ITEM_SQL,
                    tenant_id,
                    user_id,
                    product_id,
                )
                .await?
                {
                    return Ok(FuelItemCarried::AlreadyCarried(held));
                }
                let inserted =
                    Self::insert_fuel_item(&mut tx, tenant_id, user_id, Some(product_id), None, note)
                        .await?;
                let Some(id) = inserted else {
                    drop(tx);
                    return self
                        .colliding_fuel_item(FIND_LIVE_PRODUCT_ITEM_SQL, tenant_id, user_id, product_id)
                        .await;
                };
                tx.commit()
                    .await
                    .map_err(|e| fuel_error("commit an item to", e))?;
                Ok(FuelItemCarried::Added(id))
            }

            async fn add_pending_fuel_item(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                proposal_id: Uuid,
                note: Option<&str>,
            ) -> AppResult<FuelItemCarried> {
                use sqlx::Row as _;
                let row = sqlx::query(PROPOSAL_LINK_SQL)
                    .bind($ids::bind(proposal_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| fuel_error("look up a proposal in", e))?
                    .ok_or_else(|| AppError::not_found(format!("Fuel proposal {proposal_id}")))?;
                let status: String = row
                    .try_get("status")
                    .map_err(|e| fuel_error("decode status of", e))?;
                match (parse_status(&status)?, $ids::read_opt(&row, "product_id")?) {
                    (ProposalStatus::Rejected, _) => Err(AppError::invalid_input(
                        "That fuel proposal was rejected and cannot be carried",
                    )),
                    (ProposalStatus::Approved, Some(product_id)) => {
                        self.add_fuel_product_item(tenant_id, user_id, product_id, note)
                            .await
                    }
                    (ProposalStatus::Approved, None) => Err(AppError::database(
                        "An approved fuel proposal names no product",
                    )),
                    (ProposalStatus::Pending, _) => {
                        let mut tx = self.begin_fuel_item_write(tenant_id, user_id).await?;
                        // Hold the proposal while the item is written: an
                        // approval claims it with an UPDATE, so it waits for
                        // this commit and its relink then sees the item. Had
                        // it been decided since the read above, nothing is
                        // written and the decided proposal is carried instead.
                        let held = sqlx::query(HOLD_PENDING_PROPOSAL_SQL)
                            .bind($ids::bind(proposal_id))
                            .execute(&mut *tx)
                            .await
                            .map_err(|e| fuel_error("hold a proposal in", e))?
                            .rows_affected();
                        if held != 1 {
                            drop(tx);
                            return self
                                .add_pending_fuel_item(tenant_id, user_id, proposal_id, note)
                                .await;
                        }
                        if let Some(held) = Self::find_live_fuel_item(
                            &mut tx,
                            FIND_LIVE_PENDING_ITEM_SQL,
                            tenant_id,
                            user_id,
                            proposal_id,
                        )
                        .await?
                        {
                            return Ok(FuelItemCarried::AlreadyCarried(held));
                        }
                        let inserted = Self::insert_fuel_item(
                            &mut tx,
                            tenant_id,
                            user_id,
                            None,
                            Some(proposal_id),
                            note,
                        )
                        .await?;
                        let Some(id) = inserted else {
                            drop(tx);
                            return self
                                .colliding_fuel_item(
                                    FIND_LIVE_PENDING_ITEM_SQL,
                                    tenant_id,
                                    user_id,
                                    proposal_id,
                                )
                                .await;
                        };
                        tx.commit()
                            .await
                            .map_err(|e| fuel_error("commit an item to", e))?;
                        Ok(FuelItemCarried::Added(id))
                    }
                }
            }

            async fn file_and_carry_fuel_proposal(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                proposal: &NewFuelProposal,
                note: Option<&str>,
            ) -> AppResult<FuelItemCarried> {
                check_gtin(proposal.gtin.as_deref())?;
                let mut tx = self.begin_fuel_item_write(tenant_id, user_id).await?;
                // The same product filed again while its first filing is still
                // under review is already carried: decided under the lock, so
                // two concurrent filings cannot both pass and queue it twice.
                let held = sqlx::query(
                    $crate::repositories::fuel_products::FIND_LIVE_PENDING_FILING_SQL,
                )
                .bind($ids::bind(tenant_id.as_uuid()))
                .bind($ids::bind(user_id))
                .bind(fuel_natural_key(&proposal.brand, &proposal.name, &proposal.variant))
                .bind(proposal.form.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| fuel_error("find a pending filing in", e))?;
                if let Some(row) = held {
                    return Ok(FuelItemCarried::AlreadyCarried($ids::read(&row, "id")?));
                }
                let proposal_id = Self::insert_fuel_proposal(&mut tx, proposal, None).await?;
                // The cap is counted inside this transaction: refused, the
                // transaction drops and the proposal goes with it.
                let id = Self::insert_fuel_item(
                    &mut tx,
                    tenant_id,
                    user_id,
                    None,
                    Some(proposal_id),
                    note,
                )
                .await?
                .ok_or_else(|| {
                    AppError::database("A just-filed fuel proposal was already carried")
                })?;
                tx.commit()
                    .await
                    .map_err(|e| fuel_error("commit a filed item to", e))?;
                Ok(FuelItemCarried::Added(id))
            }

            async fn list_fuel_items(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
            ) -> AppResult<Vec<AthleteFuelItem>> {
                let rows = sqlx::query(LIST_ITEMS_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| fuel_error("list items in", e))?;
                rows.iter().map(Self::fuel_item_row).collect()
            }

            async fn remove_fuel_item(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                item_id: Uuid,
            ) -> AppResult<bool> {
                let removed = sqlx::query(REMOVE_ITEM_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(item_id))
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| fuel_error("remove an item from", e))?
                    .rows_affected();
                Ok(removed == 1)
            }
        }
    };
}
pub(crate) use impl_fuel_product_repositories;
