// ABOUTME: What each athlete carries, emitted per backend by macro: add, file-and-carry, list and remove fuel items
// ABOUTME: Every statement filters on tenant_id and user_id; invoked by impl_fuel_product_repositories (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Emit [`super::AthleteFuelItemRepository`] for one backend type. `$ids` is
/// that backend's uuid codec ([`crate::repositories::uuid_columns`]). The
/// helpers it calls (`begin_fuel_item_write`, `insert_fuel_item`, …) are
/// emitted by [`super::impl_fuel_product_repositories`], which invokes this.
macro_rules! impl_athlete_fuel_items {
    ($ty:ty, $ids:ident) => {
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
                let inserted = Self::insert_fuel_item(
                    &mut tx,
                    tenant_id,
                    user_id,
                    Some(product_id),
                    None,
                    note,
                )
                .await?;
                let Some(id) = inserted else {
                    drop(tx);
                    return self
                        .colliding_fuel_item(
                            FIND_LIVE_PRODUCT_ITEM_SQL,
                            tenant_id,
                            user_id,
                            product_id,
                        )
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
                let held =
                    sqlx::query($crate::repositories::fuel_products::FIND_LIVE_PENDING_FILING_SQL)
                        .bind($ids::bind(tenant_id.as_uuid()))
                        .bind($ids::bind(user_id))
                        .bind(fuel_natural_key(
                            &proposal.brand,
                            &proposal.name,
                            &proposal.variant,
                        ))
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
pub(crate) use impl_athlete_fuel_items;
