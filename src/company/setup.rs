//! First-run company setup: the curated rosters, and the rules that keep a
//! proposed roster sane.
//!
//! A brand-new company has no roster, and until now the console papered over
//! that with a fabricated twelve-agent starter team that existed only in the
//! browser. First-run setup replaces it: three questions, then four to six
//! agents actually created on the host. See
//! `docs/spec/runtime/company-setup.md`.
//!
//! ## Why the templates live here and not in the harness
//!
//! Everything in this module is deterministic and model-free, and that is the
//! point. `src/harness/` is entirely behind the non-default `openhuman`
//! feature, which CI's default lane never compiles — so a template table that
//! lived there would ship untested.
//!
//! The templates are **not** what a company normally gets. When a model is
//! wired it designs the team from the operator's own answers
//! (`crate::harness::roster_build`), and these curated rosters do two narrower
//! jobs:
//!
//! * **The floor.** No credential, a timeout, an unreadable answer — every
//!   failure lands here, so a company with no API key still gets a real
//!   industry team rather than an empty page. That is what makes the
//!   never-strand rule (decision D3) cheap to honour.
//! * **A quality bar.** The matched roster goes into the prompt as a reference
//!   for naming and phrasing, so a generated team reads like a written one.
//!
//! [`validate_roster`] is the other half, and the load-bearing one: it applies
//! the same bounds to a generated roster and a curated one, so the rules that
//! keep a team workable are a boundary rather than a request in a prompt.

use serde::{Deserialize, Serialize};

/// What the operator told us during setup.
///
/// Stored on the [`CompanyRecord`](crate::ports::types::CompanyRecord) so Phase
/// 2 (workflows) can build from answers already given rather than asking a
/// second time. Three free-text fields on purpose: people describe a business
/// in sentences, and a picker with twelve checkboxes collects less.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupAnswers {
    /// "What kind of company are you setting up?"
    #[serde(default)]
    pub industry: String,
    /// "What team do you need?" — free text alongside the pre-ticked roster.
    #[serde(default)]
    pub team_hint: String,
    /// "What are you trying to automate?" — the answer that becomes each
    /// agent's mandate in Phase 1, and the workflows in Phase 2.
    #[serde(default)]
    pub automate: String,
}

/// One agent a setup pass proposes. Not yet created — the console turns each of
/// these into a `POST {scope}/team` call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedAgent {
    /// The short display name, e.g. `Meta Ads`.
    pub name: String,
    /// The full title, e.g. `Meta Ads Specialist`.
    pub role: String,
    /// What this agent owns, in the operator's terms.
    pub description: String,
}

/// A curated agent inside a [`RosterTemplate`]. Static so the table costs no
/// allocation until a template is actually chosen.
#[derive(Clone, Copy, Debug)]
pub struct TemplateAgent {
    pub name: &'static str,
    pub role: &'static str,
    pub description: &'static str,
}

/// A hand-written starting roster for one kind of business.
#[derive(Clone, Copy, Debug)]
pub struct RosterTemplate {
    /// Stable identifier, returned to the console and worth logging.
    pub key: &'static str,
    /// Human label, e.g. `E-commerce`.
    pub label: &'static str,
    /// Lowercase substrings that select this template. Matched against the
    /// answers, not against a fixed vocabulary the operator has to guess.
    pub keywords: &'static [&'static str],
    pub agents: &'static [TemplateAgent],
}

impl RosterTemplate {
    /// This template's agents as owned, proposal-shaped rows.
    pub fn proposed(&self) -> Vec<ProposedAgent> {
        self.agents
            .iter()
            .map(|a| ProposedAgent {
                name: a.name.to_string(),
                role: a.role.to_string(),
                description: a.description.to_string(),
            })
            .collect()
    }
}

/// The fewest agents a setup pass may land.
///
/// Below this the team page reads as thin — the failure the whole feature
/// exists to fix. A short roster is topped up from its template rather than
/// shipped, so the floor holds even when a model returns one agent.
pub const MIN_AGENTS: usize = 4;

/// The most agents a setup pass may land. Beyond this a new operator is being
/// handed clutter to tidy rather than a team to work with.
pub const MAX_AGENTS: usize = 6;

/// The longest mandate a card should carry. A model asked for a one-line
/// mandate will occasionally write a paragraph; the roster card has one line
/// for it, so the cap belongs on the data rather than on the CSS.
pub const MAX_DESCRIPTION: usize = 200;

const ECOMMERCE: RosterTemplate = RosterTemplate {
    key: "ecommerce",
    label: "E-commerce",
    keywords: &[
        "ecommerce",
        "e-commerce",
        "online store",
        "shopify",
        "woocommerce",
        "amazon",
        "etsy",
        "dropship",
        "retail",
        "merch",
        "storefront",
        "inventory",
        "fulfilment",
        "fulfillment",
        "dispatch",
        "homeware",
        "apparel",
    ],
    agents: &[
        TemplateAgent {
            name: "Meta Ads",
            role: "Meta Ads Specialist",
            description: "Runs paid campaigns, budgets, and creative testing.",
        },
        TemplateAgent {
            name: "SEO",
            role: "SEO Specialist",
            description: "Product listings, organic traffic, and search rankings.",
        },
        TemplateAgent {
            name: "Logistics",
            role: "Logistics Coordinator",
            description: "Dispatch, tracking, and returns.",
        },
        TemplateAgent {
            name: "Ops",
            role: "Operations Manager",
            description: "Keeps the rest of the team moving and unblocks them.",
        },
        TemplateAgent {
            name: "Accounts",
            role: "Accountant",
            description: "Reconciliation, margins, and spend.",
        },
    ],
};

const CONTENT: RosterTemplate = RosterTemplate {
    key: "content",
    label: "Content & creator",
    keywords: &[
        "content",
        "creator",
        "influencer",
        "youtube",
        "instagram",
        "tiktok",
        "newsletter",
        "podcast",
        "blog",
        "video",
        "social media",
        "audience",
        "publishing",
    ],
    agents: &[
        TemplateAgent {
            name: "Strategy",
            role: "Content Strategist",
            description: "Decides what to publish, and when.",
        },
        TemplateAgent {
            name: "Writer",
            role: "Writer",
            description: "Drafts posts, scripts, and captions.",
        },
        TemplateAgent {
            name: "Editor",
            role: "Editor",
            description: "Reviews everything before it goes out.",
        },
        TemplateAgent {
            name: "Social",
            role: "Social Media Manager",
            description: "Schedules posts and works the comments.",
        },
        TemplateAgent {
            name: "Analyst",
            role: "Analytics Analyst",
            description: "Measures what landed and reports back.",
        },
    ],
};

const AGENCY: RosterTemplate = RosterTemplate {
    key: "agency",
    label: "Agency",
    keywords: &[
        "agency",
        "marketing agency",
        "client",
        "clients",
        "campaign",
        "branding",
        "creative studio",
        "design studio",
        "retainer",
    ],
    agents: &[
        TemplateAgent {
            name: "Accounts",
            role: "Account Manager",
            description: "Owns the client relationship and the brief.",
        },
        TemplateAgent {
            name: "Creative",
            role: "Creative Director",
            description: "Holds the concept and the creative bar.",
        },
        TemplateAgent {
            name: "Copy",
            role: "Copywriter",
            description: "Writes ads, pages, and campaign copy.",
        },
        TemplateAgent {
            name: "Media",
            role: "Paid Media Buyer",
            description: "Plans and runs paid acquisition.",
        },
        TemplateAgent {
            name: "Analyst",
            role: "Analytics Analyst",
            description: "Reports performance back to the client.",
        },
    ],
};

const CONSULTING: RosterTemplate = RosterTemplate {
    key: "consulting",
    label: "Consulting",
    keywords: &[
        "consulting",
        "consultancy",
        "advisory",
        "strategy",
        "research firm",
        "diligence",
        "analysis",
        "deck",
        "report writing",
    ],
    agents: &[
        TemplateAgent {
            name: "Engagement",
            role: "Engagement Manager",
            description: "Runs the engagement and keeps it on scope.",
        },
        TemplateAgent {
            name: "Research",
            role: "Research Analyst",
            description: "Gathers facts, sources, and context.",
        },
        TemplateAgent {
            name: "Modelling",
            role: "Financial Analyst",
            description: "Builds the models and sanity-checks the numbers.",
        },
        TemplateAgent {
            name: "Decks",
            role: "Deck Builder",
            description: "Turns findings into something presentable.",
        },
        TemplateAgent {
            name: "Writer",
            role: "Report Writer",
            description: "Drafts the written deliverable.",
        },
    ],
};

const SOFTWARE: RosterTemplate = RosterTemplate {
    key: "software",
    label: "Software",
    keywords: &[
        "software",
        "saas",
        "app",
        "product company",
        "startup",
        "platform",
        "api",
        "developer",
        "engineering",
        "b2b",
    ],
    agents: &[
        TemplateAgent {
            name: "Product",
            role: "Product Manager",
            description: "Decides what gets built, and in what order.",
        },
        TemplateAgent {
            name: "Engineer",
            role: "Software Engineer",
            description: "Builds and ships the product.",
        },
        TemplateAgent {
            name: "QA",
            role: "QA Engineer",
            description: "Tests changes before they reach anyone.",
        },
        TemplateAgent {
            name: "Design",
            role: "Product Designer",
            description: "Creates the interface and holds the brand.",
        },
        TemplateAgent {
            name: "Support",
            role: "Support Specialist",
            description: "Answers customers and closes the loop.",
        },
    ],
};

/// The roster for a business none of the others describe.
///
/// Deliberately last in [`TEMPLATES`] and reachable only by falling through:
/// it is what a keyword miss lands on, and it is still a real team.
const GENERIC: RosterTemplate = RosterTemplate {
    key: "generic",
    label: "General business",
    keywords: &[],
    agents: &[
        TemplateAgent {
            name: "Ops",
            role: "Operations Lead",
            description: "Keeps work moving and unblocks the team.",
        },
        TemplateAgent {
            name: "Research",
            role: "Researcher",
            description: "Gathers facts, sources, and context.",
        },
        TemplateAgent {
            name: "Writer",
            role: "Writer",
            description: "Drafts copy, docs, and outbound messages.",
        },
        TemplateAgent {
            name: "Analyst",
            role: "Analyst",
            description: "Measures performance and reports back.",
        },
        TemplateAgent {
            name: "Support",
            role: "Support Specialist",
            description: "Answers customers and closes the loop.",
        },
    ],
};

/// Every curated roster, most specific first. [`GENERIC`] is last because it
/// matches nothing and is only ever reached as a fallback.
pub const TEMPLATES: &[RosterTemplate] =
    &[ECOMMERCE, CONTENT, AGENCY, CONSULTING, SOFTWARE, GENERIC];

/// The template that best fits these answers, or [`GENERIC`] when none does.
///
/// `industry` is weighted above `automate` because it is the question actually
/// asking what the business *is*; the automation answer only breaks ties.
/// Without the weighting, an e-commerce operator who mentions "social media
/// posts" would be staffed as a content studio — the automation list names
/// tasks, not the business doing them.
pub fn match_template(answers: &SetupAnswers) -> &'static RosterTemplate {
    let industry = answers.industry.to_lowercase();
    let secondary = format!(
        "{} {}",
        answers.team_hint.to_lowercase(),
        answers.automate.to_lowercase()
    );

    let mut best: Option<(&'static RosterTemplate, usize)> = None;
    for template in TEMPLATES {
        let score: usize = template
            .keywords
            .iter()
            .map(|kw| {
                // Three points for naming the business, one for merely
                // mentioning the domain in a task list.
                usize::from(industry.contains(kw)) * 3 + usize::from(secondary.contains(kw))
            })
            .sum();
        if score == 0 {
            continue;
        }
        // Strictly greater, so an earlier (more specific) template holds a tie.
        if best.is_none_or(|(_, seen)| score > seen) {
            best = Some((template, score));
        }
    }
    best.map(|(template, _)| template).unwrap_or(&GENERIC)
}

/// A roster a setup pass is offering the operator, and where it came from.
///
/// Carries its provenance because the console says so out loud — decision D2 is
/// that everything setup builds is presented as a starting point, and "we picked
/// the e-commerce team" is a more honest thing to show than a roster that
/// appears from nowhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosterProposal {
    pub agents: Vec<ProposedAgent>,
    /// The [`RosterTemplate::key`] whose reference team framed this proposal.
    /// Reported for either source: it is what the model was shown as a quality
    /// bar, and what a failure fell back to.
    pub template_key: &'static str,
    /// Who wrote this team.
    pub source: RosterSource,
}

/// Who wrote a proposed roster.
///
/// Replaces an earlier `generated: bool`, which was accurate and read as a lie.
/// `generated = true` meant "a model answered the call" — but with the whole
/// roster still assembled from canned strings it was taken to mean "a model
/// wrote these words", which it did not. Naming the source makes the difference
/// unmissable, and lets the console say which one an operator is looking at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RosterSource {
    /// A model designed this team from the operator's own answers.
    Model,
    /// The curated team for this kind of business, shipped whole because no
    /// model was reachable, its answer could not be read, or what it returned
    /// was too thin to be a company. Never blended with a model's answer —
    /// see [`validate_roster`].
    Fallback,
}

impl RosterSource {
    /// The wire spelling the console reads.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Fallback => "fallback",
        }
    }
}

/// The proposal for these answers with no model involved: the matched template,
/// validated.
///
/// This is both the fast path (no inference credential wired) and the floor
/// every other path falls back to, which is why it lives here rather than
/// beside the pass that polishes it.
pub fn template_proposal(answers: &SetupAnswers) -> RosterProposal {
    let template = match_template(answers);
    RosterProposal {
        agents: validate_roster(template.proposed()),
        template_key: template.key,
        source: RosterSource::Fallback,
    }
}

/// A role's identity for de-duplication: lowercase alphanumerics, everything
/// else collapsed to a single `-`.
fn role_slug(role: &str) -> String {
    let mut slug = String::with_capacity(role.len());
    let mut pending_dash = false;
    for ch in role.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(ch.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    slug
}

/// Truncates a mandate to [`MAX_DESCRIPTION`] on a word boundary where one is
/// near, so a long answer reads as a sentence rather than a severed word.
fn clamp_description(description: &str) -> String {
    let trimmed = description.trim();
    if trimmed.chars().count() <= MAX_DESCRIPTION {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(MAX_DESCRIPTION).collect();
    let boundary = cut.rfind(' ').unwrap_or(cut.len());
    // Only honour the boundary if it keeps most of the text; a string with one
    // space near the start would otherwise be cut to almost nothing.
    let kept = if boundary > MAX_DESCRIPTION / 2 {
        &cut[..boundary]
    } else {
        cut.as_str()
    };
    format!("{}…", kept.trim_end_matches([' ', ',', ';', '.']))
}

/// Brings a proposed roster inside the rules every roster obeys, whoever
/// produced it.
///
/// Applied to **model output and template output alike**, so there is one
/// definition of a well-formed roster rather than one per producer:
///
/// * fields trimmed; a blank name falls back to the role;
/// * an entry with no role is dropped — it names nobody;
/// * mandates clamped to [`MAX_DESCRIPTION`];
/// * duplicate roles collapsed (first wins), so a model that repeats itself
///   cannot land two teammates who share one job;
/// * truncated to [`MAX_AGENTS`].
///
/// ## It does not top a short roster up, and used to
///
/// An earlier version padded anything under [`MIN_AGENTS`] with agents from the
/// matched template. It produced exactly the outcome it was meant to prevent: a
/// yoga studio asked for bookings and retention, the pass returned three agents,
/// and the fourth teammate the operator was shown was a **Content Strategist**
/// — from a template they had never seen, for work they had not mentioned. The
/// padding was invisible in the result, so the roster read as though a model had
/// chosen it.
///
/// Three relevant teammates beat four with one stranger in them. A roster too
/// thin to be a company is now the *caller's* decision, made by comparing
/// against [`MIN_AGENTS`] and falling back to the curated team **whole** — so an
/// operator is always looking at one authored team or the other, never a blend
/// of both. See [`crate::harness::roster_build`].
pub fn validate_roster(proposed: Vec<ProposedAgent>) -> Vec<ProposedAgent> {
    let mut seen: Vec<String> = Vec::new();
    let mut roster: Vec<ProposedAgent> = Vec::new();

    let push = |agent: ProposedAgent, roster: &mut Vec<ProposedAgent>, seen: &mut Vec<String>| {
        let role = agent.role.trim();
        if role.is_empty() || roster.len() >= MAX_AGENTS {
            return;
        }
        let slug = role_slug(role);
        if slug.is_empty() || seen.contains(&slug) {
            return;
        }
        let name = agent.name.trim();
        seen.push(slug);
        roster.push(ProposedAgent {
            name: if name.is_empty() {
                role.to_string()
            } else {
                name.to_string()
            },
            role: role.to_string(),
            description: clamp_description(&agent.description),
        });
    };

    for agent in proposed {
        push(agent, &mut roster, &mut seen);
    }
    roster
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answers(industry: &str, automate: &str) -> SetupAnswers {
        SetupAnswers {
            industry: industry.to_string(),
            team_hint: String::new(),
            automate: automate.to_string(),
        }
    }

    fn agent(role: &str) -> ProposedAgent {
        ProposedAgent {
            name: role.to_string(),
            role: role.to_string(),
            description: "does the thing".to_string(),
        }
    }

    /// The spec's worked example: "I sell homeware online" must staff the
    /// e-commerce team, mandate-for-mandate.
    #[test]
    fn the_worked_example_lands_the_ecommerce_roster() {
        let picked = match_template(&answers(
            "E-commerce — I sell homeware online",
            "Social media posts, Meta ads, generating my reports, order dispatch",
        ));
        assert_eq!(picked.key, "ecommerce");
        let roles: Vec<&str> = picked.agents.iter().map(|a| a.role).collect();
        assert!(roles.contains(&"Logistics Coordinator"), "{roles:?}");
        assert!(roles.contains(&"Meta Ads Specialist"), "{roles:?}");
    }

    /// The weighting that keeps the automation list from overruling the
    /// business. An e-commerce operator naming social posts is still running a
    /// shop, and staffing them as a content studio would leave nobody on
    /// dispatch.
    #[test]
    fn the_industry_answer_outweighs_the_automation_list() {
        let picked = match_template(&answers(
            "online store selling homeware",
            "instagram, tiktok, youtube, podcast, newsletter, blog",
        ));
        assert_eq!(picked.key, "ecommerce");
    }

    /// The automation answer still decides when the industry says nothing
    /// recognisable — it is the tiebreak, not dead weight.
    #[test]
    fn the_automation_answer_breaks_a_tie() {
        let picked = match_template(&answers("just me", "scheduling my youtube uploads"));
        assert_eq!(picked.key, "content");
    }

    /// A miss must land a real team, not nothing. This is decision D3's cheap
    /// half: the never-strand fallback is a curated roster.
    #[test]
    fn an_unrecognised_business_still_gets_a_real_team() {
        let picked = match_template(&answers("zzzz qqqq", ""));
        assert_eq!(picked.key, "generic");
        assert!(picked.agents.len() >= MIN_AGENTS);
    }

    /// Every curated roster must itself satisfy the rules it is the fallback
    /// for. A template that could not pass validation would be a floor that
    /// does not hold.
    #[test]
    fn every_template_is_within_its_own_bounds() {
        for template in TEMPLATES {
            let count = template.agents.len();
            assert!(
                (MIN_AGENTS..=MAX_AGENTS).contains(&count),
                "{} has {count} agents",
                template.key
            );
            let validated = validate_roster(template.proposed());
            assert_eq!(
                validated.len(),
                count,
                "{} lost agents to validation",
                template.key
            );
            for a in template.agents {
                assert!(
                    !a.role.trim().is_empty(),
                    "{} has a blank role",
                    template.key
                );
                assert!(
                    a.description.chars().count() <= MAX_DESCRIPTION,
                    "{} has an over-long mandate",
                    template.key
                );
            }
        }
    }

    /// Template keys are how a proposal reports which roster it came from, so
    /// two templates sharing one would make that report ambiguous.
    #[test]
    fn template_keys_are_unique() {
        let mut keys: Vec<&str> = TEMPLATES.iter().map(|t| t.key).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), before, "duplicate template key");
    }

    #[test]
    fn an_over_long_roster_is_truncated() {
        let long: Vec<ProposedAgent> = (0..12).map(|i| agent(&format!("Role {i}"))).collect();
        assert_eq!(validate_roster(long).len(), MAX_AGENTS);
    }

    /// **No padding.** A short roster comes back short, so nothing an operator is
    /// shown was quietly borrowed from a template they never saw.
    ///
    /// The regression this guards is concrete: a yoga studio's pass returned
    /// three agents, validation padded it to four from the `content` template,
    /// and the fourth teammate on screen was a Content Strategist — rendered
    /// identically to the three the operator had actually asked for. Deciding
    /// what to do about a thin roster belongs to the caller, which falls back to
    /// the curated team **whole**.
    #[test]
    fn a_short_roster_is_left_short_rather_than_padded() {
        let roster = validate_roster(vec![agent("Meta Ads Specialist")]);
        assert_eq!(roster.len(), 1, "validation must not invent teammates");
        assert_eq!(roster[0].role, "Meta Ads Specialist");
    }

    /// Two teammates sharing one job is the failure the operator would have to
    /// clean up by hand, so near-miss spellings collapse too.
    #[test]
    fn duplicate_roles_collapse_however_they_are_spelled() {
        let roster = validate_roster(vec![
            agent("SEO Specialist"),
            agent("seo  specialist"),
            agent("SEO-Specialist"),
        ]);
        let seo = roster
            .iter()
            .filter(|a| role_slug(&a.role) == "seo-specialist")
            .count();
        assert_eq!(seo, 1, "{roster:?}");
    }

    #[test]
    fn a_roleless_entry_is_dropped_and_a_blank_name_falls_back_to_the_role() {
        let roster = validate_roster(vec![
            ProposedAgent {
                name: "Ghost".into(),
                role: "   ".into(),
                description: String::new(),
            },
            ProposedAgent {
                name: "  ".into(),
                role: "Data Analyst".into(),
                description: String::new(),
            },
        ]);
        assert!(roster.iter().all(|a| !a.role.trim().is_empty()));
        let analyst = roster.iter().find(|a| a.role == "Data Analyst").unwrap();
        assert_eq!(analyst.name, "Data Analyst");
    }

    /// A model asked for one line occasionally writes a paragraph. The card has
    /// one line for it, so the cap is on the data.
    #[test]
    fn an_over_long_mandate_is_clamped() {
        let essay = "word ".repeat(200);
        let roster = validate_roster(vec![ProposedAgent {
            name: "A".into(),
            role: "Analyst".into(),
            description: essay,
        }]);
        let clamped = &roster[0].description;
        assert!(clamped.chars().count() <= MAX_DESCRIPTION + 1, "{clamped}");
        assert!(clamped.ends_with('…'), "{clamped}");
    }

    /// Validation of nothing is nothing. The floor is the caller's business now,
    /// and `template_proposal` is where an operator with no usable model still
    /// gets a real team.
    #[test]
    fn validation_of_an_empty_roster_stays_empty() {
        assert!(validate_roster(Vec::new()).is_empty());
    }

    /// The honest fallback: a full curated team, labelled as such, for the
    /// offline path and every failure path.
    #[test]
    fn the_fallback_is_a_whole_curated_team_and_says_so() {
        let proposal = template_proposal(&answers("I sell homeware online", ""));
        assert_eq!(proposal.template_key, "ecommerce");
        assert_eq!(proposal.source, RosterSource::Fallback);
        assert_eq!(proposal.source.as_str(), "fallback");
        assert!(
            proposal.agents.len() >= MIN_AGENTS,
            "a fallback must be a workable team, got {}",
            proposal.agents.len()
        );
        // Whole, not blended: every row is the template's own.
        let curated: Vec<&str> = ECOMMERCE.agents.iter().map(|a| a.role).collect();
        for a in &proposal.agents {
            assert!(
                curated.contains(&a.role.as_str()),
                "{} is not curated",
                a.role
            );
        }
    }

    /// The answers ride on the company record, so they must survive the round
    /// trip the record makes through its store.
    #[test]
    fn answers_round_trip_through_serde() {
        let answers = SetupAnswers {
            industry: "E-commerce".into(),
            team_hint: "plus customer support".into(),
            automate: "Meta ads, order dispatch".into(),
        };
        let json = serde_json::to_string(&answers).expect("serialize");
        assert_eq!(
            serde_json::from_str::<SetupAnswers>(&json).expect("deserialize"),
            answers
        );
        // And a record written before setup existed still loads.
        assert_eq!(
            serde_json::from_str::<SetupAnswers>("{}").expect("empty"),
            SetupAnswers::default()
        );
    }
}
