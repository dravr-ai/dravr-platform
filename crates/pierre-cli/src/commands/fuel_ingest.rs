// ABOUTME: `pierre-cli fuel-ingest` — files fuel-product proposals from brands' Shopify storefronts, labels read by OCR
// ABOUTME: Checked-in brand list, discovery, a paid-read ledger and a listing key before the LLM, the label read, the gate, a counted summary (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `pierre-cli fuel-ingest [--brand <slug>]… [--limit <n>] [--dry-run]`.
//!
//! The weekly `*-fuel-ingest-job` Cloud Run job runs it. For each brand in
//! [`BRANDS`] it lists the storefront's `/products.json`, keeps the fuel
//! packages, splits each into the flavours its photos show, downloads every
//! photo up to [`shopify::MAX_IMAGES_PER_CANDIDATE`], and reads the label.
//! Nothing enters the catalogue here: a proposal waits for an operator's
//! approval.
//!
//! - **Never pay twice.** Every read the job pays for is recorded in
//!   `fuel_ingest_reads` under its [`fuel_read_hash`] — the page URL, every
//!   photo sent in order, the `fuel_panel_extraction` prompt text and the
//!   configured model — whatever it came to: a proposal, an illegible label,
//!   a duplicate, an unchanged product or an answer that did not parse. A
//!   hash on file is skipped before the call, so an unchanged weekly run
//!   costs only the downloads, and a new photo, prompt or model reads again.
//!   A provider error or a tier that cannot see images is not recorded: the
//!   next run tries again.
//! - **One read per product, before the read.** A listing's
//!   [`shopify::listing_key`] (brand, title without pack-size words, flavour)
//!   names the product whatever pack it is sold in: Näak sells one drink mix
//!   as a "Bag" and a "Single Serve" listing, whose labels print the name
//!   differently. A listing a live proposal holds, or whose product a pending
//!   update holds, is not read.
//! - **A catalogued product is re-read, never re-filed.** A read of a product
//!   the catalogue holds — by its listing, else by the brand, name and
//!   variant its label prints — is compared with the product's current
//!   facts: the same facts file nothing (`unchanged`), different ones file a
//!   pending update of that product, whose approval supersedes its version.
//!   A read whose label a live proposal holds files nothing (`duplicate`). An
//!   athlete's typed filing never holds a label: the read is better evidence.
//! - **The label is the only source.** The read goes through the configured
//!   chain; an answer from a tier that does not read images is discarded
//!   ([`extraction::read_label`]). A label with no readable panel files
//!   nothing (`illegible`): it never reaches the queue.
//! - **Flagged, never clamped.** The gate's violation codes travel on the
//!   proposal so the reviewer sees why.
//! - **Identity.** The brand is the table's display name, so one brand is one
//!   spelling; name and variant are what the label prints, falling back to
//!   the listing title and its flavour.
//! - **What the reviewer sees.** Every photo sent, in order; the reader's
//!   notes, the ingredients and the glucose + fructose claim; and the job's
//!   notes when the photos were not tied to one flavour or the proposal
//!   updates a catalogued product.
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
    fuel_natural_key, fuel_read_hash, FuelFacts, FuelIngestOutcome, FuelKeyHolder, NewFuelProposal,
    ProposalSource, ProposalSourceImage,
};
use pierre_core::redaction::redact_url;
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use pierre_llm::config::LlmChain;
use pierre_llm::stage::LlmStage;
use pierre_llm::ChatProvider;
use serde_json::json;
use tracing::{error, info, warn};
use uuid::Uuid;

use extraction::{label_request, map_reading, read_label, ReadFailure, Reading};
use shopify::{
    download_image, fetch_fuel_products, label_candidates, listing_key, LabelCandidate, LabelImage,
    ShopifyProduct, MAX_READ_BYTES,
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
    /// Reads filed as a new product's proposal.
    proposed: usize,
    /// Reads filed as an update of a catalogued product whose facts changed.
    proposed_update: usize,
    /// Reads that found no readable panel; nothing filed.
    illegible: usize,
    /// Reads whose product a live proposal already holds; nothing filed.
    duplicate: usize,
    /// Reads of a catalogued product whose facts are unchanged; nothing filed.
    unchanged: usize,
    /// Answers that were not the prompt's object; recorded, nothing filed.
    parse_failed: usize,
    /// Candidates whose read hash is on file: no call.
    skipped_already_read: usize,
    /// Candidates whose listing a live proposal holds: no call.
    skipped_listing_held: usize,
    skipped_no_photo: usize,
    llm_calls: usize,
    /// Prompt tokens the answers were billed for, as the providers report.
    prompt_tokens: u64,
    /// Completion tokens the answers were billed for.
    completion_tokens: u64,
    discarded_no_vision: usize,
    /// Reads that errored on every tier (not recorded: retried next run).
    read_failed: usize,
    /// Candidates that failed outside the read (download, database).
    failed: usize,
}

impl Tally {
    /// Log the summary the job-failure alert's reader starts from.
    fn log(&self, dry_run: bool, brands: usize) {
        info!(
            dry_run,
            brands,
            brands_failed = self.brands_failed,
            listed = self.products_listed,
            products_seen = self.products_seen,
            candidates = self.candidates,
            proposed = self.proposed,
            proposed_update = self.proposed_update,
            illegible = self.illegible,
            duplicate = self.duplicate,
            unchanged = self.unchanged,
            parse_failed = self.parse_failed,
            skipped_already_read = self.skipped_already_read,
            skipped_listing_held = self.skipped_listing_held,
            skipped_no_photo = self.skipped_no_photo,
            llm_calls = self.llm_calls,
            prompt_tokens = self.prompt_tokens,
            completion_tokens = self.completion_tokens,
            discarded_no_vision = self.discarded_no_vision,
            read_failed = self.read_failed,
            failed = self.failed,
            "fuel ingestion finished"
        );
    }

    /// The run's verdict: an error when labels were sent and none was read,
    /// when every candidate failed before or after its read (the database
    /// down, every photo download refused), or when no brand could be listed.
    fn verdict(&self, brands: usize) -> AppResult<()> {
        let unread = self.discarded_no_vision + self.read_failed + self.parse_failed;
        if self.llm_calls > 0 && unread >= self.llm_calls {
            return Err(AppError::external_service(
                "llm",
                format!(
                    "every label read failed ({} sent, {} discarded for no vision, {} errored, {} unparseable)",
                    self.llm_calls, self.discarded_no_vision, self.read_failed, self.parse_failed
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

/// The model a label read is sent to, as the read hash names it: the head
/// provider and the model its `fuel_panel_extraction` stage runs on.
fn configured_model(provider: &ChatProvider) -> String {
    let model = provider
        .stage_model(LlmStage::FuelPanelExtraction)
        .unwrap_or_else(|| provider.default_model());
    format!("{}={model}", provider.name())
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

/// What a listing's key says before any photo is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Listing {
    /// Nothing holds it.
    Unfiled,
    /// The catalogue holds its product: a new photo set is read and compared.
    Catalogued(Uuid),
    /// A live proposal holds it (a new product, or an update of the
    /// catalogued one): not read until that proposal is decided.
    Held,
}

/// What a read came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filing {
    /// A proposal for a product the catalogue does not hold.
    New,
    /// A proposal updating this catalogued product: its facts changed.
    Update(Uuid),
    /// No readable panel; nothing filed.
    Illegible,
    /// What holds the product already, or `None` for an earlier candidate of
    /// this dry run; nothing filed.
    Duplicate(Option<FuelKeyHolder>),
    /// This catalogued product's current facts are what the label states.
    Unchanged(Uuid),
    /// The answer was not the prompt's object.
    ParseFailed,
}

impl Filing {
    /// The outcome the read is recorded with.
    const fn outcome(self) -> FuelIngestOutcome {
        match self {
            Self::New | Self::Update(_) => FuelIngestOutcome::Proposed,
            Self::Illegible => FuelIngestOutcome::Illegible,
            Self::Duplicate(_) => FuelIngestOutcome::Duplicate,
            Self::Unchanged(_) => FuelIngestOutcome::Unchanged,
            Self::ParseFailed => FuelIngestOutcome::ParseFailed,
        }
    }

    /// How the summary and the dry run name it.
    const fn label(self) -> &'static str {
        match self {
            Self::New => "proposed",
            Self::Update(_) => "proposed_update",
            other => other.outcome().as_str(),
        }
    }
}

/// What a legible read of catalogued `product` comes to: nothing while a
/// pending update holds it, nothing when its `current` facts are what was
/// `read`, else an update whose approval supersedes its version.
fn against_product(
    product: Uuid,
    pending: Option<Uuid>,
    current: &FuelFacts,
    read: &FuelFacts,
) -> Filing {
    match pending {
        Some(update) => Filing::Duplicate(Some(FuelKeyHolder::Proposal(update))),
        None if current == read => Filing::Unchanged(product),
        None => Filing::Update(product),
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

/// `notes` with the ingestion job's `note` after them.
fn with_job_note(notes: Option<String>, note: &str) -> String {
    notes.map_or_else(
        || format!("Ingestion job: {note}"),
        |notes| format!("{notes}\nIngestion job: {note}"),
    )
}

/// Everything one run holds.
struct Run {
    repos: Arc<RepositoryRegistry>,
    provider: ChatProvider,
    prompt: String,
    /// [`configured_model`], hashed into every read.
    model: String,
    http: reqwest::Client,
    /// `--dry-run`: print what would be filed, write nothing.
    preview: bool,
    /// The natural keys and listing keys this dry run would have filed a
    /// proposal under, so a run that writes nothing still sees a second
    /// listing of one product.
    filed_keys: HashSet<String>,
    filed_listings: HashSet<String>,
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
    let provider = ChatProvider::from_chain(&LlmChain::from_env()).await?;
    let model = configured_model(&provider);
    info!(model = %model, "label reads go to the configured model");
    let mut run = Run {
        repos: Arc::clone(db.repositories()),
        provider,
        prompt: PromptRegistry::new().fuel_panel_extraction_prompt(),
        model,
        http: shopify::http_client()?,
        preview: args.dry_run,
        filed_keys: HashSet::new(),
        filed_listings: HashSet::new(),
        tally: Tally::default(),
    };
    for brand in &brands {
        run.brand(brand, args.limit).await;
    }
    let t = &run.tally;
    t.log(run.preview, brands.len());
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

    /// What holds `listing` before any photo is read.
    async fn listing(&self, listing: &str) -> AppResult<Listing> {
        if self.preview && self.filed_listings.contains(listing) {
            return Ok(Listing::Held);
        }
        let catalogue = &self.repos.fuel_catalogue;
        Ok(match catalogue.fuel_listing_holder(listing).await? {
            None => Listing::Unfiled,
            Some(FuelKeyHolder::Proposal(_)) => Listing::Held,
            Some(FuelKeyHolder::Product(product)) => {
                if catalogue.pending_fuel_update(product).await?.is_some() {
                    Listing::Held
                } else {
                    Listing::Catalogued(product)
                }
            }
        })
    }

    /// The candidate's photos, every one up to [`MAX_READ_BYTES`] together:
    /// read from the last, where the panel usually is, and kept in the
    /// store's order.
    async fn photos(&self, candidate: &LabelCandidate) -> AppResult<Vec<LabelImage>> {
        let mut images = Vec::with_capacity(candidate.image_urls.len());
        let mut bytes = 0;
        for src in candidate.image_urls.iter().rev() {
            let Some(image) = download_image(&self.http, src).await? else {
                continue;
            };
            if bytes + image.bytes.len() > MAX_READ_BYTES {
                warn!(
                    page = %redact_url(&candidate.page_url),
                    bytes = image.bytes.len(),
                    "photo left out: the read's photos are over the byte budget"
                );
                continue;
            }
            bytes += image.bytes.len();
            images.push(image);
        }
        images.reverse();
        Ok(images)
    }

    /// The photos to read and their read hash, or `None` (counted) when no
    /// photo was usable or this read is already on file.
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
        let sent: Vec<&[u8]> = images.iter().map(|image| image.bytes.as_slice()).collect();
        let hash = fuel_read_hash(&candidate.page_url, &sent, &self.prompt, &self.model);
        if self
            .repos
            .fuel_catalogue
            .fuel_ingest_read_seen(&hash)
            .await?
        {
            info!(brand = brand.slug, %page, "already read with these photos, prompt and model, skipped");
            self.tally.skipped_already_read += 1;
            return Ok(None);
        }
        Ok(Some((images, hash)))
    }

    /// Read the label `images` show, counting the call and its tokens.
    /// `Ok(None)` (logged) when nothing usable came back.
    async fn read(
        &mut self,
        brand: &Brand,
        candidate: &LabelCandidate,
        images: &[LabelImage],
    ) -> AppResult<Option<Result<Reading, AppError>>> {
        let page = redact_url(&candidate.page_url);
        let request = label_request(&self.provider, &self.prompt, images)?;
        self.tally.llm_calls += 1;
        let answer = read_label(&self.provider, &request, &candidate.page_url).await;
        if let Some(usage) = &answer.usage {
            self.tally.prompt_tokens += u64::from(usage.prompt_tokens);
            self.tally.completion_tokens += u64::from(usage.completion_tokens);
        }
        match answer.result {
            Ok(read) => Ok(Some(Ok(read))),
            Err(ReadFailure::NoVision) => {
                self.tally.discarded_no_vision += 1;
                Ok(None)
            }
            Err(ReadFailure::Provider(e)) => {
                error!(brand = brand.slug, %page, "label read failed: {e}");
                self.tally.read_failed += 1;
                Ok(None)
            }
            Err(ReadFailure::Unparseable(e)) => {
                error!(brand = brand.slug, %page, "label answer refused: {e}");
                Ok(Some(Err(e)))
            }
        }
    }

    /// What `read` comes to. `listed` is the catalogued product the
    /// candidate's listing names, which outranks the label's own key: a
    /// label prints a product's name differently on each listing.
    async fn filing(
        &self,
        read: &Reading,
        proposal: &NewFuelProposal,
        listed: Option<Uuid>,
    ) -> AppResult<Filing> {
        if !read.label.legible {
            return Ok(Filing::Illegible);
        }
        let catalogue = &self.repos.fuel_catalogue;
        let holder = match listed {
            Some(product) => Some(FuelKeyHolder::Product(product)),
            None => {
                catalogue
                    .fuel_key_holder(&proposal.brand, &proposal.name, &proposal.variant)
                    .await?
            }
        };
        match holder {
            None => {
                let key = fuel_natural_key(&proposal.brand, &proposal.name, &proposal.variant);
                if self.preview && self.filed_keys.contains(&key) {
                    return Ok(Filing::Duplicate(None));
                }
                Ok(Filing::New)
            }
            Some(FuelKeyHolder::Proposal(_)) => Ok(Filing::Duplicate(holder)),
            Some(FuelKeyHolder::Product(product)) => {
                let pending = catalogue.pending_fuel_update(product).await?;
                let current = catalogue.get_fuel_product(product).await?.ok_or_else(|| {
                    AppError::database(format!(
                        "Catalogued fuel product {product} has no current version"
                    ))
                })?;
                Ok(against_product(
                    product,
                    pending,
                    &current.current.facts,
                    &proposal.facts,
                ))
            }
        }
    }

    async fn candidate(&mut self, brand: &Brand, candidate: &LabelCandidate) -> AppResult<()> {
        let listing = listing_key(brand.slug, &candidate.title, candidate.flavour.as_deref());
        let listed = match self.listing(&listing).await? {
            Listing::Held => {
                info!(
                    brand = brand.slug,
                    page = %redact_url(&candidate.page_url),
                    %listing,
                    "a live proposal holds this listing's product, not read"
                );
                self.tally.skipped_listing_held += 1;
                return Ok(());
            }
            Listing::Catalogued(product) => Some(product),
            Listing::Unfiled => None,
        };
        let Some((images, hash)) = self.fresh_photos(brand, candidate).await? else {
            return Ok(());
        };
        let read = match self.read(brand, candidate, &images).await? {
            None => return Ok(()),
            Some(Err(_)) => {
                self.file(candidate, &hash, &listing, Filing::ParseFailed, None)
                    .await?;
                self.count(Filing::ParseFailed, None, &listing);
                return Ok(());
            }
            Some(Ok(read)) => read,
        };
        let mut proposal = proposal(brand, candidate, &read, &images, hash.clone());
        let filing = self.filing(&read, &proposal, listed).await?;
        if filing == Filing::Illegible {
            // Nothing is filed, so the log is the only place the reader's
            // reason survives.
            warn!(
                brand = brand.slug,
                page = %redact_url(&candidate.page_url),
                notes = read.label.notes.as_deref().unwrap_or("none"),
                "label read found no readable panel; nothing filed"
            );
        }
        if let Filing::Update(product) = filing {
            proposal.product_id = Some(product);
            proposal.notes = Some(with_job_note(
                proposal.notes.take(),
                &format!(
                    "Updates catalogued product {product}: the label's facts differ from its current version."
                ),
            ));
        }
        if self.preview {
            println!("{}", preview_line(&proposal, &read, filing));
        } else {
            self.file(candidate, &hash, &listing, filing, Some(&proposal))
                .await?;
        }
        self.count(filing, Some(&proposal), &listing);
        Ok(())
    }

    /// Record the read (not in a dry run), filing `proposal` when it came
    /// to one.
    async fn file(
        &self,
        candidate: &LabelCandidate,
        hash: &str,
        listing: &str,
        filing: Filing,
        proposal: Option<&NewFuelProposal>,
    ) -> AppResult<()> {
        if self.preview {
            if filing == Filing::ParseFailed {
                println!(
                    "{}",
                    json!({ "would_record": { "as": filing.label(), "source_url": candidate.page_url } })
                );
            }
            return Ok(());
        }
        let filed = proposal.filter(|_| matches!(filing, Filing::New | Filing::Update(_)));
        let id = self
            .repos
            .fuel_catalogue
            .record_fuel_ingest_read(hash, filing.outcome(), filed.map(|p| (p, listing)))
            .await?;
        info!(
            page = %redact_url(&candidate.page_url),
            proposal = ?id,
            outcome = filing.label(),
            flags = filed.map_or(0, |p| p.gate_flags.len()),
            "label read recorded"
        );
        Ok(())
    }

    /// Count `filing`, and in a dry run remember what it would have filed.
    fn count(&mut self, filing: Filing, proposal: Option<&NewFuelProposal>, listing: &str) {
        let tally = &mut self.tally;
        match filing {
            Filing::New => tally.proposed += 1,
            Filing::Update(_) => tally.proposed_update += 1,
            Filing::Illegible => tally.illegible += 1,
            Filing::Duplicate(_) => tally.duplicate += 1,
            Filing::Unchanged(_) => tally.unchanged += 1,
            Filing::ParseFailed => tally.parse_failed += 1,
        }
        if let (Filing::New | Filing::Update(_), Some(proposal)) = (filing, proposal) {
            self.filed_listings.insert(listing.to_owned());
            self.filed_keys.insert(fuel_natural_key(
                &proposal.brand,
                &proposal.name,
                &proposal.variant,
            ));
        }
    }
}

/// The proposal a read label files, with every photo sent, in order.
fn proposal(
    brand: &Brand,
    candidate: &LabelCandidate,
    read: &Reading,
    images: &[LabelImage],
    hash: String,
) -> NewFuelProposal {
    let label = &read.label;
    let mapped = map_reading(label, &candidate.page_url);
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
        gate_flags: mapped.gate_flags,
        notes: reviewer_notes(label.notes.as_deref(), candidate.photo_note()),
        ingredients_text: label
            .ingredients_text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned),
        multiple_transportable_stated: Some(label.multiple_transportable_stated),
        proposed_by_user_id: None,
    }
}

/// The `--dry-run` line for a read.
fn preview_line(proposal: &NewFuelProposal, read: &Reading, filing: Filing) -> serde_json::Value {
    let held_by = match filing {
        Filing::Duplicate(Some(holder)) => json!(holder),
        Filing::Duplicate(None) => json!("an earlier listing in this run"),
        Filing::Update(product) | Filing::Unchanged(product) => {
            json!(FuelKeyHolder::Product(product))
        }
        Filing::New | Filing::Illegible | Filing::ParseFailed => serde_json::Value::Null,
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
            "photos_sent": proposal.images.len(),
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
    fn every_outcome_is_recorded_under_its_stored_spelling() {
        let product = Uuid::nil();
        let cases = [
            (Filing::New, FuelIngestOutcome::Proposed, "proposed"),
            (
                Filing::Update(product),
                FuelIngestOutcome::Proposed,
                "proposed_update",
            ),
            (Filing::Illegible, FuelIngestOutcome::Illegible, "illegible"),
            (
                Filing::Duplicate(None),
                FuelIngestOutcome::Duplicate,
                "duplicate",
            ),
            (
                Filing::Unchanged(product),
                FuelIngestOutcome::Unchanged,
                "unchanged",
            ),
            (
                Filing::ParseFailed,
                FuelIngestOutcome::ParseFailed,
                "parse_failed",
            ),
        ];
        for (filing, outcome, label) in cases {
            assert_eq!(filing.outcome(), outcome, "{filing:?}");
            assert_eq!(filing.label(), label, "{filing:?}");
        }
    }

    #[test]
    fn a_job_note_follows_the_readers() {
        assert_eq!(with_job_note(None, "b"), "Ingestion job: b");
        assert_eq!(
            with_job_note(Some("Label reader: a".to_owned()), "b"),
            "Label reader: a\nIngestion job: b"
        );
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
        // Reads on file and held listings are no call, and no failure.
        assert!(Tally {
            skipped_already_read: 5,
            skipped_listing_held: 2,
            ..Tally::default()
        }
        .verdict(1)
        .is_ok());
        // Illegible, duplicate and unchanged reads were read.
        assert!(Tally {
            candidates: 3,
            llm_calls: 3,
            illegible: 1,
            duplicate: 1,
            unchanged: 1,
            ..Tally::default()
        }
        .verdict(1)
        .is_ok());
        // Answers that never parse are failed reads.
        assert!(Tally {
            candidates: 2,
            llm_calls: 2,
            parse_failed: 1,
            read_failed: 1,
            ..Tally::default()
        }
        .verdict(1)
        .is_err());
        assert!(Tally {
            brands_failed: 2,
            ..Tally::default()
        }
        .verdict(2)
        .is_err());
    }

    fn read() -> Reading {
        let mut label =
            extraction::parse_reading(include_str!("fuel_ingest/fixtures/label_reading.json"))
                .unwrap_or_else(|e| panic!("{e}"));
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
            &read(),
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
    fn a_catalogued_product_is_updated_only_when_its_facts_changed() {
        let product = Uuid::new_v4();
        let current = read().label;
        let facts = map_reading(&current, "https://naak.com/products/gel").facts;
        assert_eq!(
            against_product(product, None, &facts, &facts),
            Filing::Unchanged(product)
        );
        let mut changed = facts.clone();
        changed.sodium_mg = Some(400.0);
        assert_eq!(
            against_product(product, None, &facts, &changed),
            Filing::Update(product)
        );
        // A pending update already holds it: nothing more is filed.
        let pending = Uuid::new_v4();
        assert_eq!(
            against_product(product, Some(pending), &facts, &changed),
            Filing::Duplicate(Some(FuelKeyHolder::Proposal(pending)))
        );
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
