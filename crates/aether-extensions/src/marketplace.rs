use super::{
    invalid, validate_extension_id, validate_slug, validate_text, Capability, CapabilityKind,
    ExtensionError, ExtensionRecipe, RECIPE_VERSION,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const CATALOG_VERSION: u32 = 1;
pub const MAX_CATALOG_BYTES: usize = 2 * 1024 * 1024;
const MAX_LISTINGS: usize = 256;
const MAX_REVIEWS_PER_LISTING: usize = 16;
const MAX_TAGS_PER_LISTING: usize = 16;

const BUILTIN_CATALOG_JSON: &str = include_str!("../../../marketplace/girder-extensions.json");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketplaceCatalog {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub listings: Vec<MarketplaceListing>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketplaceListing {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub intent: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub reference_recipe: ExtensionRecipe,
    pub recipe_digest: String,
    pub listing_digest: String,
    pub reviews: Vec<MarketplaceReview>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketplaceReview {
    pub reviewer: String,
    pub decision: ReviewDecision,
    pub recipe_digest: String,
    pub listing_digest: String,
    pub evidence: String,
}

#[derive(Serialize)]
struct ListingDigestPayload<'a> {
    id: &'a str,
    name: &'a str,
    summary: &'a str,
    intent: &'a str,
    tags: &'a [String],
    reference_recipe: &'a ExtensionRecipe,
    recipe_digest: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Approved,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDelta {
    pub added: Vec<Capability>,
    pub removed: Vec<Capability>,
}

impl MarketplaceCatalog {
    pub fn from_json(json: &str) -> Result<Self, ExtensionError> {
        if json.len() > MAX_CATALOG_BYTES {
            return invalid(format!("catalog exceeds {MAX_CATALOG_BYTES} bytes"));
        }
        let catalog: Self = serde_json::from_str(json)?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn to_json_pretty(&self) -> Result<String, ExtensionError> {
        self.validate()?;
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn validate(&self) -> Result<(), ExtensionError> {
        if self.version != CATALOG_VERSION {
            return invalid(format!(
                "unsupported catalog version {}; expected {CATALOG_VERSION}",
                self.version
            ));
        }
        validate_extension_id(&self.id)?;
        validate_text("catalog name", &self.name, 1, 80)?;
        validate_text("catalog description", &self.description, 1, 500)?;
        if self.listings.len() > MAX_LISTINGS {
            return invalid(format!(
                "catalog contains more than {MAX_LISTINGS} listings"
            ));
        }

        let mut ids = BTreeSet::new();
        for listing in &self.listings {
            listing.validate()?;
            if !ids.insert(listing.id.as_str()) {
                return invalid(format!("duplicate marketplace listing {}", listing.id));
            }
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String, ExtensionError> {
        self.validate()?;
        Ok(sha256_hex(&serde_json::to_vec(self)?))
    }

    pub fn listing(&self, id: &str) -> Option<&MarketplaceListing> {
        self.listings.iter().find(|listing| listing.id == id)
    }

    pub fn search(&self, query: &str) -> Vec<&MarketplaceListing> {
        let terms = query
            .split_whitespace()
            .map(|term| term.to_ascii_lowercase())
            .collect::<Vec<_>>();
        let mut matches = self
            .listings
            .iter()
            .filter_map(|listing| {
                let score = if terms.is_empty() {
                    1
                } else {
                    search_score(listing, &terms)
                };
                (score > 0).then_some((score, listing))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.id.cmp(&right.id))
        });
        matches.into_iter().map(|(_, listing)| listing).collect()
    }
}

impl MarketplaceListing {
    pub fn validate(&self) -> Result<(), ExtensionError> {
        validate_extension_id(&self.id)?;
        validate_text("listing name", &self.name, 1, 80)?;
        validate_text("listing summary", &self.summary, 1, 500)?;
        validate_text("listing intent", &self.intent, 1, 2_000)?;
        if self.tags.len() > MAX_TAGS_PER_LISTING {
            return invalid(format!(
                "listing {} contains more than {MAX_TAGS_PER_LISTING} tags",
                self.id
            ));
        }
        let mut tags = BTreeSet::new();
        for tag in &self.tags {
            validate_slug("marketplace tag", tag, 32)?;
            if !tags.insert(tag.as_str()) {
                return invalid(format!("listing {} contains duplicate tags", self.id));
            }
        }

        self.reference_recipe.validate()?;
        if self.reference_recipe.version != RECIPE_VERSION {
            return invalid(format!(
                "listing {} uses an unsupported recipe version",
                self.id
            ));
        }
        if self.reference_recipe.id != self.id {
            return invalid(format!(
                "listing {} does not match reference recipe id {}",
                self.id, self.reference_recipe.id
            ));
        }
        let digest = self.reference_recipe.digest()?;
        validate_digest("listing recipe digest", &self.recipe_digest)?;
        if self.recipe_digest != digest {
            return invalid(format!(
                "listing {} recipe digest does not match its reference recipe",
                self.id
            ));
        }
        validate_digest("listing digest", &self.listing_digest)?;
        let listing_digest = self.content_digest()?;
        if self.listing_digest != listing_digest {
            return invalid(format!(
                "listing {} digest does not match its reviewed content",
                self.id
            ));
        }
        if self.reviews.is_empty() || self.reviews.len() > MAX_REVIEWS_PER_LISTING {
            return invalid(format!(
                "listing {} must contain 1 to {MAX_REVIEWS_PER_LISTING} reviews",
                self.id
            ));
        }

        let mut reviewers = BTreeSet::new();
        let mut approved = false;
        for review in &self.reviews {
            review.validate(&digest, &listing_digest)?;
            if !reviewers.insert(review.reviewer.as_str()) {
                return invalid(format!(
                    "listing {} contains duplicate reviewer {}",
                    self.id, review.reviewer
                ));
            }
            approved |= review.decision == ReviewDecision::Approved;
        }
        if !approved {
            return invalid(format!(
                "listing {} has no review approving the exact recipe digest",
                self.id
            ));
        }
        Ok(())
    }

    pub fn capability_delta(
        &self,
        adapted: &ExtensionRecipe,
    ) -> Result<CapabilityDelta, ExtensionError> {
        self.validate()?;
        adapted.validate()?;
        if adapted.id != self.id {
            return invalid(format!(
                "adapted recipe id {} does not match marketplace listing {}",
                adapted.id, self.id
            ));
        }
        let reference = capabilities_by_canonical_json(&self.reference_recipe.capabilities)?;
        let candidate = capabilities_by_canonical_json(&adapted.capabilities)?;
        Ok(CapabilityDelta {
            added: candidate
                .iter()
                .filter(|(encoded, _)| !reference.contains_key(*encoded))
                .map(|(_, capability)| capability.clone())
                .collect(),
            removed: reference
                .iter()
                .filter(|(encoded, _)| !candidate.contains_key(*encoded))
                .map(|(_, capability)| capability.clone())
                .collect(),
        })
    }

    pub fn approved_review_count(&self) -> usize {
        self.reviews
            .iter()
            .filter(|review| review.decision == ReviewDecision::Approved)
            .count()
    }

    pub fn digest(&self) -> Result<String, ExtensionError> {
        self.validate()?;
        self.content_digest()
    }

    fn content_digest(&self) -> Result<String, ExtensionError> {
        let payload = ListingDigestPayload {
            id: &self.id,
            name: &self.name,
            summary: &self.summary,
            intent: &self.intent,
            tags: &self.tags,
            reference_recipe: &self.reference_recipe,
            recipe_digest: &self.recipe_digest,
        };
        Ok(sha256_hex(&serde_json::to_vec(&payload)?))
    }
}

impl MarketplaceReview {
    fn validate(
        &self,
        expected_recipe_digest: &str,
        expected_listing_digest: &str,
    ) -> Result<(), ExtensionError> {
        validate_extension_id(&self.reviewer)?;
        validate_digest("review recipe digest", &self.recipe_digest)?;
        validate_digest("review listing digest", &self.listing_digest)?;
        validate_text("review evidence", &self.evidence, 1, 1_000)?;
        if self.recipe_digest != expected_recipe_digest {
            return invalid(format!(
                "review by {} is not bound to the listing recipe digest",
                self.reviewer
            ));
        }
        if self.listing_digest != expected_listing_digest {
            return invalid(format!(
                "review by {} is not bound to the full marketplace listing",
                self.reviewer
            ));
        }
        Ok(())
    }
}

impl CapabilityDelta {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }

    pub fn added_kinds(&self) -> Vec<CapabilityKind> {
        self.added.iter().map(Capability::kind).collect()
    }

    pub fn removed_kinds(&self) -> Vec<CapabilityKind> {
        self.removed.iter().map(Capability::kind).collect()
    }
}

pub fn builtin_catalog() -> Result<MarketplaceCatalog, ExtensionError> {
    MarketplaceCatalog::from_json(BUILTIN_CATALOG_JSON)
}

pub fn adaptation_system_prompt() -> &'static str {
    r#"Return exactly one JSON object and no prose or markdown. Adapt the reviewed declarative Girder extension listing to the current project. Preserve the listing's intent and extension id. The reference recipe is context, not authorization. Request only capabilities necessary for the adapted recipe. Never emit executable plugin code. Supported recipe version is 1. Every contribution and projection must satisfy the recipe's declared capabilities. The result will be validated and shown with an exact capability delta before a human may approve its SHA-256 digest."#
}

pub fn marketplace_project_context(graph: &aether_graph::SemanticGraph) -> serde_json::Value {
    let mut nodes = graph
        .nodes()
        .map(|node| {
            serde_json::json!({
                "path": &node.path,
                "kind": format!("{:?}", node.kind),
                "language": &node.language,
            })
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        left["path"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["path"].as_str().unwrap_or_default())
    });
    nodes.truncate(128);
    serde_json::json!({
        "node_count": graph.node_count(),
        "edge_count": graph.edge_count(),
        "sample_nodes": nodes,
    })
}

fn capabilities_by_canonical_json(
    capabilities: &[Capability],
) -> Result<BTreeMap<String, Capability>, ExtensionError> {
    capabilities
        .iter()
        .map(|capability| {
            serde_json::to_string(capability)
                .map(|encoded| (encoded, capability.clone()))
                .map_err(ExtensionError::from)
        })
        .collect()
}

fn search_score(listing: &MarketplaceListing, terms: &[String]) -> usize {
    let id = listing.id.to_ascii_lowercase();
    let name = listing.name.to_ascii_lowercase();
    let summary = listing.summary.to_ascii_lowercase();
    let intent = listing.intent.to_ascii_lowercase();
    let tags = listing.tags.join(" ").to_ascii_lowercase();
    terms
        .iter()
        .map(|term| {
            usize::from(id.contains(term)) * 8
                + usize::from(name.contains(term)) * 6
                + usize::from(tags.contains(term)) * 4
                + usize::from(summary.contains(term)) * 2
                + usize::from(intent.contains(term))
        })
        .sum()
}

fn validate_digest(label: &str, digest: &str) -> Result<(), ExtensionError> {
    if digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        Ok(())
    } else {
        invalid(format!("{label} must be a 64-character SHA-256 hex digest"))
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Contribution, PanelLocation, PanelView};

    fn listing() -> MarketplaceListing {
        let recipe = ExtensionRecipe {
            version: RECIPE_VERSION,
            id: "dev.bitcode.impact-browser".into(),
            name: "Impact Browser".into(),
            description: "Shows semantic impact in a project-aware panel.".into(),
            intent: "show callers affected by a selected symbol".into(),
            capabilities: vec![Capability::ReadGraph, Capability::ContributeUi],
            contributions: vec![Contribution::Panel {
                id: "impact".into(),
                title: "Impact".into(),
                location: PanelLocation::Right,
                view: PanelView::GraphQuery {
                    question: "what is impacted by the selected node?".into(),
                },
            }],
            projections: Vec::new(),
        };
        let digest = recipe.digest().unwrap();
        let mut listing = MarketplaceListing {
            id: recipe.id.clone(),
            name: recipe.name.clone(),
            summary: recipe.description.clone(),
            intent: recipe.intent.clone(),
            tags: vec!["impact".into(), "graph".into()],
            reference_recipe: recipe,
            recipe_digest: digest.clone(),
            listing_digest: String::new(),
            reviews: Vec::new(),
        };
        let listing_digest = listing.content_digest().unwrap();
        listing.listing_digest = listing_digest.clone();
        listing.reviews = vec![MarketplaceReview {
            reviewer: "org.bitcode.security".into(),
            decision: ReviewDecision::Approved,
            recipe_digest: digest,
            listing_digest,
            evidence: "Capabilities and graph query were manually reviewed.".into(),
        }];
        listing
    }

    fn catalog() -> MarketplaceCatalog {
        MarketplaceCatalog {
            version: CATALOG_VERSION,
            id: "org.bitcode.catalog".into(),
            name: "Girder Catalog".into(),
            description: "Reviewed declarative extension intents.".into(),
            listings: vec![listing()],
        }
    }

    #[test]
    fn catalog_round_trip_and_digest_are_deterministic() {
        let catalog = catalog();
        let json = catalog.to_json_pretty().unwrap();
        let decoded = MarketplaceCatalog::from_json(&json).unwrap();

        assert_eq!(decoded, catalog);
        assert_eq!(decoded.digest().unwrap(), catalog.digest().unwrap());
        assert_eq!(catalog.digest().unwrap().len(), 64);
    }

    #[test]
    fn listing_and_reviews_bind_the_exact_reference_recipe() {
        let mut listing = listing();
        listing.reference_recipe.description.push_str(" changed");

        assert!(listing
            .validate()
            .unwrap_err()
            .to_string()
            .contains("digest"));
    }

    #[test]
    fn review_binding_rejects_changed_listing_intent() {
        let mut listing = listing();
        listing.intent = "run an unrelated validation command".into();

        assert!(listing
            .validate()
            .unwrap_err()
            .to_string()
            .contains("reviewed content"));
    }

    #[test]
    fn duplicate_listing_and_reviewer_ids_are_rejected() {
        let mut catalog = catalog();
        catalog.listings.push(catalog.listings[0].clone());
        assert!(catalog
            .validate()
            .unwrap_err()
            .to_string()
            .contains("duplicate marketplace listing"));

        let mut listing = listing();
        listing.reviews.push(listing.reviews[0].clone());
        assert!(listing
            .validate()
            .unwrap_err()
            .to_string()
            .contains("duplicate reviewer"));
    }

    #[test]
    fn catalog_unknown_fields_and_oversized_inputs_are_rejected() {
        let mut value = serde_json::to_value(catalog()).unwrap();
        value.as_object_mut().unwrap().insert(
            "download_url".into(),
            serde_json::json!("file:///tmp/plugin"),
        );
        assert!(MarketplaceCatalog::from_json(&value.to_string()).is_err());

        let oversized = " ".repeat(MAX_CATALOG_BYTES + 1);
        assert!(MarketplaceCatalog::from_json(&oversized)
            .unwrap_err()
            .to_string()
            .contains("exceeds"));
    }

    #[test]
    fn listing_requires_an_approval_bound_to_current_content() {
        let mut listing = listing();
        listing.reviews[0].decision = ReviewDecision::Rejected;

        assert!(listing
            .validate()
            .unwrap_err()
            .to_string()
            .contains("no review approving"));
    }

    #[test]
    fn search_is_ranked_and_deterministic() {
        let catalog = catalog();
        assert_eq!(catalog.search("impact graph")[0].id, listing().id);
        assert!(catalog.search("debugger").is_empty());
    }

    #[test]
    fn capability_delta_compares_exact_scopes() {
        let listing = listing();
        let mut adapted = listing.reference_recipe.clone();
        adapted.capabilities.push(Capability::ReadProject {
            paths: vec!["src/**".into()],
        });

        let delta = listing.capability_delta(&adapted).unwrap();
        assert_eq!(delta.added_kinds(), vec![CapabilityKind::ReadProject]);
        assert!(delta.removed.is_empty());
    }

    #[test]
    fn adaptation_cannot_change_the_reviewed_extension_identity() {
        let listing = listing();
        let mut adapted = listing.reference_recipe.clone();
        adapted.id = "dev.bitcode.different-extension".into();

        assert!(listing
            .capability_delta(&adapted)
            .unwrap_err()
            .to_string()
            .contains("does not match marketplace listing"));
    }

    #[test]
    fn builtin_catalog_is_valid_and_reviewed() {
        let catalog = builtin_catalog().unwrap();
        assert!(!catalog.listings.is_empty());
        assert!(catalog
            .listings
            .iter()
            .all(|listing| listing.approved_review_count() > 0));
    }

    #[test]
    fn project_context_is_sorted_and_bounded() {
        let mut graph = aether_graph::SemanticGraph::new();
        for index in (0..140).rev() {
            graph.upsert_node(aether_graph::Node::new(
                aether_graph::NodeKind::Function,
                format!("function-{index:03}"),
                format!("crate::function_{index:03}"),
            ));
        }

        let context = marketplace_project_context(&graph);
        let nodes = context["sample_nodes"].as_array().unwrap();
        assert_eq!(context["node_count"], 140);
        assert_eq!(nodes.len(), 128);
        assert_eq!(nodes[0]["path"], "crate::function_000");
        assert_eq!(nodes[127]["path"], "crate::function_127");
    }
}
