// ABOUTME: `pierre-cli fuel-ingest` — files fuel-product proposals from brands' Shopify storefronts, labels read by OCR
// ABOUTME: Checked-in brand list, discovery, dedupe by source hash, the embacle label read, the gate, a counted summary (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `pierre-cli fuel-ingest [--brand <slug>]… [--limit <n>] [--dry-run]`.
//!
//! The weekly `*-fuel-ingest-job` Cloud Run job runs it. For each brand in
//! [`BRANDS`] it lists the storefront's `/products.json`, keeps the fuel
//! packages, splits each into the flavours its photos show, downloads the last
//! [`shopify::MAX_IMAGES_PER_CANDIDATE`] photos, and files one proposal per
//! package read. Nothing enters the catalogue here: a proposal waits for an
//! operator's approval.
//!
//! - **Dedupe before the LLM.** The source hash
//!   ([`pierre_core::models::proposal_source_hash`]) is taken over the page
//!   URL and every photo sent, in order; a hash already on file skips the
//!   package without a call, so a weekly rerun costs only the downloads. Every
//!   read that costs a call is therefore filed, whatever it found.
//! - **The label is the only source.** The read goes through the configured
//!   chain with the contremaitre `fuel_panel_extraction` prompt; an answer
//!   from a tier that does not read images is discarded
//!   ([`extraction::read_label`]).
//! - **Flagged, never clamped.** The gate's violation codes travel on the
//!   proposal so the reviewer sees why. A label with no readable panel is
//!   filed pending with the [`ILLEGIBLE_FLAG`], for a one-click rejection.
//! - **One proposal per product.** A read whose brand, name and variant a
//!   catalogue product or a live proposal already holds (Näak sells one drink
//!   mix as a "Bag" and a "Single Serve" listing) is filed already rejected
//!   as a duplicate, without its photos, so it never reaches the queue.
//! - **Identity.** The brand is the table's display name, so one brand is one
//!   spelling; name and variant are what the label prints, falling back to
//!   the listing title and its flavour.
//! - **What the reviewer sees.** Every photo sent, in order; the reader's
//!   notes, the ingredients and the glucose + fructose claim; and the job's
//!   note when the photos were not tied to one flavour.
//!
//! The run fails (non-zero exit, which the job-failure alert counts) when
//! labels were sent and not one read came back, when every candidate failed
//! outside the read, or when no brand could be listed.

mod extraction;
mod shopify;

use std::collections::HashSet;
use std::sync::Arc;

use pierre_contremaitre::registry::PromptRegistry;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    fuel_natural_key, proposal_source_hash, FuelKeyHolder, NewFuelProposal, ProposalSource,
    ProposalSourceImage, ILLEGIBLE_FLAG,
};
use pierre_core::redaction::redact_url;
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use pierre_llm::config::LlmChain;
use pierre_llm::ChatProvider;
use serde_json::json;
use tracing::{error, info, warn};

use extraction::{label_request, map_reading, read_label, ReadFailure, Reading};
use shopify::{
    download_image, fetch_fuel_products, label_candidates, LabelCandidate, LabelImage,
    ShopifyProduct,
};

/// A brand the job reads, and how its storefront is filtered to fuel.
#[derive(Debug)]
pub(crate) struct Brand {
    /// The `--brand` value.
    pub slug: &'static str,
    /// The brand as the catalogue names it.
    pub display: &'static str,
    /// The storefront base URL; `/products.json` answers under it.
    pub store: &'static str,
    /// Whole-word keywords, any of which in a listing's type, tags or title
    /// marks it as fuel (the shared exclusions in [`shopify`] still apply).
    pub keywords: &'static [&'static str],
}

/// The brands the job reads. Each store was checked to be a Shopify
/// storefront serving `/products.json` on 2026-10-29.
pub(crate) const BRANDS: [Brand; 4] = [
    Brand {
        slug: "naak",
        display: "Näak",
        store: "https://naak.com",
        keywords: &[
            "gel",
            "gels",
            "drink mix",
            "drink mixes",
            "bar",
            "bars",
            "waffle",
            "waffles",
            "puree",
            "purees",
        ],
    },
    Brand {
        slug: "tailwind",
        display: "Tailwind Nutrition",
        store: "https://tailwindnutrition.com",
        keywords: &["endurance fuel", "high carb fuel", "rapid hydration"],
    },
    Brand {
        slug: "skratch",
        display: "Skratch Labs",
        store: "https://www.skratchlabs.com",
        keywords: &[
            "drink mix",
            "energy chews",
            "energy bar",
            "sport fuel",
            "electrolyte",
        ],
    },
    Brand {
        slug: "gu",
        display: "GU Energy Labs",
        store: "https://guenergy.com",
        keywords: &[
            "gel",
            "energy gel",
            "liquid energy",
            "chews",
            "drink mix",
            "hydration tabs",
            "drink tabs",
            "capsules",
            "waffle",
        ],
    },
];

/// `pierre-cli fuel-ingest` arguments.
#[derive(clap::Args)]
pub(crate) struct FuelIngestArgs {
    /// A brand slug to read (repeatable); every brand when omitted.
    #[arg(long = "brand", value_name = "SLUG")]
    pub brands: Vec<String>,

    /// Fuel products read per brand; every one when omitted.
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,

    /// Read the labels and print what would be proposed; write nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// What the run did, counted.
#[derive(Debug, Default, PartialEq, Eq)]
struct Tally {
    brands_failed: usize,
    products_listed: usize,
    products_seen: usize,
    candidates: usize,
    proposed: usize,
    /// Reads filed pending with the illegible flag.
    filed_illegible: usize,
    /// Reads filed already rejected: their natural key was held.
    filed_duplicate: usize,
    skipped_duplicate: usize,
    skipped_no_photo: usize,
    llm_calls: usize,
    discarded_no_vision: usize,
    /// Reads that errored on every tier or answered with no usable object.
    read_failed: usize,
    /// Candidates that failed outside the read (download, database).
    failed: usize,
}

impl Tally {
    /// The run's verdict: an error when labels were sent and none was read,
    /// when every candidate failed before or after its read (the database
    /// down, every photo download refused), or when no brand could be listed.
    fn verdict(&self, brands: usize) -> AppResult<()> {
        let unread = self.discarded_no_vision + self.read_failed;
        if self.llm_calls > 0 && unread >= self.llm_calls {
            return Err(AppError::external_service(
                "llm",
                format!(
                    "every label read failed ({} sent, {} discarded for no vision, {} failed)",
                    self.llm_calls, self.discarded_no_vision, self.read_failed
                ),
            ));
        }
        if self.candidates > 0 && self.failed == self.candidates {
            return Err(AppError::external_service(
                "fuel-ingest",
                format!(
                    "every label candidate failed outside the read ({} of {}: downloads or the database)",
                    self.failed, self.candidates
                ),
            ));
        }
        if brands > 0 && self.brands_failed == brands {
            return Err(AppError::external_service(
                "shopify",
                "no brand storefront could be listed",
            ));
        }
        Ok(())
    }
}

/// The brands `slugs` names, every brand when it names none.
///
/// # Errors
/// Returns an error naming a slug the table does not hold.
fn selected_brands(slugs: &[String]) -> AppResult<Vec<&'static Brand>> {
    if slugs.is_empty() {
        return Ok(BRANDS.iter().collect());
    }
    slugs
        .iter()
        .map(|slug| {
            BRANDS
                .iter()
                .find(|brand| brand.slug.eq_ignore_ascii_case(slug))
                .ok_or_else(|| {
                    let known: Vec<_> = BRANDS.iter().map(|brand| brand.slug).collect();
                    AppError::invalid_input(format!(
                        "unknown brand '{slug}'; known: {}",
                        known.join(", ")
                    ))
                })
        })
        .collect()
}

/// The dedupe hash of a package: its page URL and every photo sent, in order.
fn source_hash(page_url: &str, images: &[LabelImage]) -> String {
    let sent: Vec<&[u8]> = images.iter().map(|image| image.bytes.as_slice()).collect();
    proposal_source_hash(Some(page_url), &sent)
}

/// The reviewer's notes on a read: the label reader's, then the job's own
/// about the photos, each named.
fn reviewer_notes(reader: Option<&str>, job: Option<String>) -> Option<String> {
    let reader = reader
        .map(str::trim)
        .filter(|note| !note.is_empty())
        .map(|note| format!("Label reader: {note}"));
    let job = job.map(|note| format!("Ingestion job: {note}"));
    match (reader, job) {
        (Some(reader), Some(job)) => Some(format!("{reader}\n{job}")),
        (one, other) => one.or(other),
    }
}

/// How a read is filed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filing {
    /// A pending proposal for review.
    Proposal,
    /// A pending proposal flagged illegible.
    Illegible,
    /// Already rejected: what holds its natural key, or `None` for a
    /// candidate earlier in this dry run.
    Duplicate(Option<FuelKeyHolder>),
}

impl Filing {
    const fn label(self) -> &'static str {
        match self {
            Self::Proposal => "proposal",
            Self::Illegible => "illegible",
            Self::Duplicate(_) => "duplicate",
        }
    }
}

/// The text a label printed, or `fallback` when it printed none.
fn printed_or(printed: Option<&str>, fallback: &str) -> String {
    printed
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

/// Everything one run holds.
struct Run {
    repos: Arc<RepositoryRegistry>,
    provider: ChatProvider,
    prompt: String,
    http: reqwest::Client,
    /// `--dry-run`: print what would be proposed, write nothing.
    preview: bool,
    /// The natural keys this run filed a proposal under, so a dry run, which
    /// writes nothing, still sees a second listing of one product.
    filed_keys: HashSet<String>,
    tally: Tally,
}

/// Run `pierre-cli fuel-ingest`.
///
/// # Errors
/// Returns an error for an unknown brand, when the database or the LLM chain
/// cannot be opened, or per [`Tally::verdict`].
pub(crate) async fn run(args: FuelIngestArgs, database_url: &str) -> AppResult<()> {
    let brands = selected_brands(&args.brands)?;
    info!(
        "Connecting to database for fuel ingestion: {}",
        redact_url(database_url)
    );
    let db = Database::init_for_seeding(database_url).await?;
    let mut run = Run {
        repos: Arc::clone(db.repositories()),
        provider: ChatProvider::from_chain(&LlmChain::from_env()).await?,
        prompt: PromptRegistry::new().fuel_panel_extraction_prompt(),
        http: shopify::http_client()?,
        preview: args.dry_run,
        filed_keys: HashSet::new(),
        tally: Tally::default(),
    };
    for brand in &brands {
        run.brand(brand, args.limit).await;
    }
    let t = &run.tally;
    info!(
        dry_run = run.preview,
        brands = brands.len(),
        brands_failed = t.brands_failed,
        listed = t.products_listed,
        products_seen = t.products_seen,
        candidates = t.candidates,
        proposed = t.proposed,
        filed_illegible = t.filed_illegible,
        filed_duplicate = t.filed_duplicate,
        skipped_duplicate = t.skipped_duplicate,
        skipped_no_photo = t.skipped_no_photo,
        llm_calls = t.llm_calls,
        discarded_no_vision = t.discarded_no_vision,
        read_failed = t.read_failed,
        failed = t.failed,
        "fuel ingestion finished"
    );
    t.verdict(brands.len())
        .inspect_err(|e| error!("fuel ingestion failed: {e}"))
}

impl Run {
    async fn brand(&mut self, brand: &Brand, limit: Option<usize>) {
        let Some(products) = self.list(brand, limit).await else {
            return;
        };
        for product in &products {
            for candidate in label_candidates(brand.store, product) {
                self.tally.candidates += 1;
                if let Err(e) = self.candidate(brand, &candidate).await {
                    error!(
                        brand = brand.slug,
                        page = %redact_url(&candidate.page_url),
                        "label candidate failed: {e}"
                    );
                    self.tally.failed += 1;
                }
            }
        }
    }

    /// The brand's fuel products, counted; `None` (logged) when the
    /// storefront could not be listed.
    async fn list(&mut self, brand: &Brand, limit: Option<usize>) -> Option<Vec<ShopifyProduct>> {
        match fetch_fuel_products(&self.http, brand, limit).await {
            Ok((listed, products)) => {
                info!(
                    brand = brand.slug,
                    listed,
                    fuel = products.len(),
                    "storefront listed"
                );
                self.tally.products_listed += listed;
                self.tally.products_seen += products.len();
                Some(products)
            }
            Err(e) => {
                error!(brand = brand.slug, "storefront listing failed: {e}");
                self.tally.brands_failed += 1;
                None
            }
        }
    }

    async fn photos(&self, candidate: &LabelCandidate) -> AppResult<Vec<LabelImage>> {
        let mut images = Vec::with_capacity(candidate.image_urls.len());
        for src in &candidate.image_urls {
            if let Some(image) = download_image(&self.http, src).await? {
                images.push(image);
            }
        }
        Ok(images)
    }

    /// The photos to read and their source hash, or `None` (counted) when no
    /// photo was usable or these photos were already proposed.
    async fn fresh_photos(
        &mut self,
        brand: &Brand,
        candidate: &LabelCandidate,
    ) -> AppResult<Option<(Vec<LabelImage>, String)>> {
        let page = redact_url(&candidate.page_url);
        let images = self.photos(candidate).await?;
        if images.is_empty() {
            warn!(brand = brand.slug, %page, "no usable photo, skipped");
            self.tally.skipped_no_photo += 1;
            return Ok(None);
        }
        let hash = source_hash(&candidate.page_url, &images);
        if self
            .repos
            .fuel_catalogue
            .fuel_proposal_exists_by_hash(&hash)
            .await?
        {
            info!(brand = brand.slug, %page, "already proposed from these photos, skipped");
            self.tally.skipped_duplicate += 1;
            return Ok(None);
        }
        Ok(Some((images, hash)))
    }

    /// The label `images` show, or `None` (counted, logged) when the read
    /// failed or came from a tier that cannot see images.
    async fn read(
        &mut self,
        brand: &Brand,
        candidate: &LabelCandidate,
        images: &[LabelImage],
    ) -> AppResult<Option<Reading>> {
        let page = redact_url(&candidate.page_url);
        let request = label_request(&self.provider, &self.prompt, images)?;
        self.tally.llm_calls += 1;
        let read = match read_label(&self.provider, &request, &candidate.page_url).await {
            Ok(read) => read,
            Err(ReadFailure::NoVision) => {
                self.tally.discarded_no_vision += 1;
                return Ok(None);
            }
            Err(ReadFailure::Provider(e) | ReadFailure::Unparseable(e)) => {
                error!(brand = brand.slug, %page, "label read failed: {e}");
                self.tally.read_failed += 1;
                return Ok(None);
            }
        };
        Ok(Some(read))
    }

    /// How `proposal` is filed: illegible, a duplicate of what holds its
    /// natural key, or a proposal.
    async fn filing(&self, read: &Reading, proposal: &NewFuelProposal) -> AppResult<Filing> {
        if !read.label.legible {
            return Ok(Filing::Illegible);
        }
        let held = self
            .repos
            .fuel_catalogue
            .fuel_key_holder(&proposal.brand, &proposal.name, &proposal.variant)
            .await?;
        if held.is_some() {
            return Ok(Filing::Duplicate(held));
        }
        let key = fuel_natural_key(&proposal.brand, &proposal.name, &proposal.variant);
        if self.preview && self.filed_keys.contains(&key) {
            return Ok(Filing::Duplicate(None));
        }
        Ok(Filing::Proposal)
    }

    async fn candidate(&mut self, brand: &Brand, candidate: &LabelCandidate) -> AppResult<()> {
        let Some((images, hash)) = self.fresh_photos(brand, candidate).await? else {
            return Ok(());
        };
        let Some(read) = self.read(brand, candidate, &images).await? else {
            return Ok(());
        };
        let proposal = proposal(brand, candidate, &read, &images, hash);
        let filing = self.filing(&read, &proposal).await?;
        if self.preview {
            println!("{}", preview_line(&proposal, &read, filing));
        } else {
            let catalogue = &self.repos.fuel_catalogue;
            let id = match filing {
                Filing::Duplicate(Some(holder)) => {
                    catalogue
                        .file_duplicate_fuel_proposal(&proposal, holder)
                        .await?
                }
                Filing::Proposal | Filing::Illegible | Filing::Duplicate(None) => {
                    catalogue.create_fuel_proposal(&proposal).await?
                }
            };
            info!(
                brand = brand.slug,
                page = %redact_url(&candidate.page_url),
                proposal = %id,
                filed_as = filing.label(),
                flags = proposal.gate_flags.len(),
                provider = read.provider,
                "fuel proposal filed"
            );
        }
        match filing {
            Filing::Proposal => {
                self.filed_keys.insert(fuel_natural_key(
                    &proposal.brand,
                    &proposal.name,
                    &proposal.variant,
                ));
                self.tally.proposed += 1;
            }
            Filing::Illegible => self.tally.filed_illegible += 1,
            Filing::Duplicate(_) => self.tally.filed_duplicate += 1,
        }
        Ok(())
    }
}

/// The proposal a read label files, with every photo sent, in order. An
/// illegible read carries the [`ILLEGIBLE_FLAG`] ahead of any gate code.
fn proposal(
    brand: &Brand,
    candidate: &LabelCandidate,
    read: &Reading,
    images: &[LabelImage],
    hash: String,
) -> NewFuelProposal {
    let label = &read.label;
    let mapped = map_reading(label, &candidate.page_url);
    let mut gate_flags = mapped.gate_flags;
    if !label.legible {
        gate_flags.insert(0, ILLEGIBLE_FLAG.to_owned());
    }
    NewFuelProposal {
        product_id: None,
        brand: brand.display.to_owned(),
        name: printed_or(label.name.as_deref(), &candidate.title),
        variant: printed_or(
            label.variant.as_deref(),
            candidate.flavour.as_deref().unwrap_or_default(),
        ),
        form: label.form,
        gtin: mapped.gtin,
        source_kind: ProposalSource::ShopifyOcr,
        source_url: Some(candidate.page_url.clone()),
        images: images
            .iter()
            .map(|image| ProposalSourceImage {
                mime: image.mime.clone(),
                bytes: image.bytes.clone(),
            })
            .collect(),
        source_hash: Some(hash),
        facts: mapped.facts,
        provenance: mapped.provenance,
        gate_flags,
        notes: reviewer_notes(label.notes.as_deref(), candidate.photo_note()),
        ingredients_text: label
            .ingredients_text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned),
        // An illegible read is no reading of the pack's claims either.
        multiple_transportable_stated: label.legible.then_some(label.multiple_transportable_stated),
        proposed_by_user_id: None,
    }
}

/// The `--dry-run` line for a read that would be filed.
fn preview_line(proposal: &NewFuelProposal, read: &Reading, filing: Filing) -> serde_json::Value {
    let held_by = match filing {
        Filing::Duplicate(Some(holder)) => json!(holder),
        Filing::Duplicate(None) => json!("an earlier listing in this run"),
        Filing::Proposal | Filing::Illegible => serde_json::Value::Null,
    };
    json!({
        "would_file": {
            "as": filing.label(),
            "held_by": held_by,
            "brand": proposal.brand,
            "name": proposal.name,
            "variant": proposal.variant,
            "form": proposal.form,
            "gtin": proposal.gtin,
            "source_url": proposal.source_url,
            "photos_stored": proposal.images.len(),
            "photo_bytes": proposal.images.iter().map(|image| image.bytes.len()).collect::<Vec<_>>(),
            "facts": proposal.facts,
            "gate_flags": proposal.gate_flags,
            "notes": proposal.notes,
            "ingredients_text": proposal.ingredients_text,
            "multiple_transportable_stated": proposal.multiple_transportable_stated,
            "label_brand": read.label.brand,
            "served_by": read.provider,
            "served_tier_position": read.position,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(bytes: &[u8]) -> LabelImage {
        LabelImage {
            mime: "image/png".to_owned(),
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn every_brand_slug_is_unique_and_selectable() {
        let all = selected_brands(&[]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(all.len(), BRANDS.len());
        let picked = selected_brands(&["GU".to_owned(), "naak".to_owned()])
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            picked.iter().map(|b| b.slug).collect::<Vec<_>>(),
            vec!["gu", "naak"]
        );
        assert!(selected_brands(&["maurten".to_owned()]).is_err());
    }

    #[test]
    fn the_source_hash_is_the_stored_one_over_every_photo_in_order() {
        let url = "https://naak.com/products/gel";
        let a = [image(b"front"), image(b"panel")];
        assert_eq!(
            source_hash(url, &a),
            proposal_source_hash(Some(url), &[b"front", b"panel"])
        );
        assert_ne!(
            source_hash(url, &a),
            source_hash(url, &[image(b"panel"), image(b"front")])
        );
    }

    #[test]
    fn the_reviewer_notes_name_who_wrote_each() {
        assert_eq!(reviewer_notes(None, None), None);
        assert_eq!(reviewer_notes(Some("  "), None), None);
        assert_eq!(
            reviewer_notes(Some("200 kcal is front-of-pack"), None).as_deref(),
            Some("Label reader: 200 kcal is front-of-pack")
        );
        assert_eq!(
            reviewer_notes(Some("a"), Some("b".to_owned())).as_deref(),
            Some("Label reader: a\nIngestion job: b")
        );
        assert_eq!(
            reviewer_notes(None, Some("b".to_owned())).as_deref(),
            Some("Ingestion job: b")
        );
    }

    #[test]
    fn the_run_fails_only_when_every_read_failed_or_no_brand_listed() {
        let mut tally = Tally {
            llm_calls: 3,
            read_failed: 2,
            discarded_no_vision: 1,
            ..Tally::default()
        };
        assert!(tally.verdict(1).is_err());
        tally.proposed = 1;
        tally.read_failed = 1;
        assert!(tally.verdict(1).is_ok());
        // A download or database failure is not a failed read.
        assert!(Tally {
            candidates: 5,
            llm_calls: 1,
            proposed: 1,
            failed: 4,
            ..Tally::default()
        }
        .verdict(1)
        .is_ok());
        // But a run where every candidate failed (the database down: even the
        // dedupe lookup errors, so no read is ever sent) filed nothing.
        assert!(Tally {
            candidates: 4,
            failed: 4,
            ..Tally::default()
        }
        .verdict(1)
        .is_err());
        // Duplicates and illegible labels are not failures; no call is not one.
        assert!(Tally {
            skipped_duplicate: 5,
            ..Tally::default()
        }
        .verdict(1)
        .is_ok());
        assert!(Tally {
            candidates: 2,
            llm_calls: 2,
            filed_illegible: 1,
            filed_duplicate: 1,
            ..Tally::default()
        }
        .verdict(1)
        .is_ok());
        assert!(Tally {
            brands_failed: 2,
            ..Tally::default()
        }
        .verdict(2)
        .is_err());
    }

    fn read(legible: bool) -> Reading {
        let mut label =
            extraction::parse_reading(include_str!("fuel_ingest/fixtures/label_reading.json"))
                .unwrap_or_else(|e| panic!("{e}"));
        label.legible = legible;
        label.notes = Some("Energy is front-of-pack".to_owned());
        Reading {
            label,
            provider: "anthropic",
            position: Some(0),
        }
    }

    fn untied_candidate() -> LabelCandidate {
        LabelCandidate {
            page_url: "https://tailwindnutrition.com/products/endurance-fuel".to_owned(),
            flavour: None,
            title: "Endurance Fuel".to_owned(),
            image_urls: vec!["a".to_owned(), "b".to_owned()],
            untied_flavours: vec!["Berry".to_owned(), "Lemon".to_owned()],
        }
    }

    #[test]
    fn a_read_files_every_photo_in_order_with_the_readers_findings() {
        let photos = [image(b"front"), image(b"panel")];
        let filed = proposal(
            &BRANDS[1],
            &untied_candidate(),
            &read(true),
            &photos,
            "hash".to_owned(),
        );
        assert_eq!(
            filed
                .images
                .iter()
                .map(|i| i.bytes.as_slice())
                .collect::<Vec<_>>(),
            vec![b"front".as_slice(), b"panel".as_slice()]
        );
        assert_eq!(filed.brand, "Tailwind Nutrition");
        assert_eq!(filed.name, "ULTRA Energy Gel");
        assert_eq!(filed.multiple_transportable_stated, Some(true));
        assert_eq!(
            filed.ingredients_text.as_deref(),
            Some("Maple syrup, maltodextrin, water, sea salt")
        );
        let notes = filed.notes.unwrap_or_default();
        assert!(
            notes.starts_with("Label reader: Energy is front-of-pack\n"),
            "{notes}"
        );
        assert!(
            notes.contains("Ingestion job: The listing offers 2 flavours (Berry, Lemon)"),
            "{notes}"
        );
        assert_eq!(filed.facts.glucose_fructose_ratio, Some(0.8));
        assert!(filed.gate_flags.is_empty(), "{:?}", filed.gate_flags);
    }

    #[test]
    fn an_illegible_read_is_filed_flagged_first() {
        let photos = [image(b"front")];
        let filed = proposal(
            &BRANDS[1],
            &untied_candidate(),
            &read(false),
            &photos,
            "hash".to_owned(),
        );
        assert_eq!(
            filed.gate_flags.first().map(String::as_str),
            Some(ILLEGIBLE_FLAG)
        );
        assert_eq!(filed.images.len(), 1);
        assert_eq!(filed.multiple_transportable_stated, None);
    }

    #[test]
    fn a_printed_identity_wins_over_the_listing() {
        assert_eq!(
            printed_or(Some(" ULTRA Gel "), "ULTRA™ Gel 200 · Maple"),
            "ULTRA Gel"
        );
        assert_eq!(printed_or(Some("  "), "Listing"), "Listing");
        assert_eq!(printed_or(None, ""), "");
    }
}
