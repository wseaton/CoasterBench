//! View models: everything the templates need, already formatted.
//!
//! Templates stay dumb on purpose (loops and layout only), so number
//! formatting, ranking and asset resolution all happen here.

use askama::Template;

pub const TAGLINE: &str = "LLMs design roller coasters. RollerCoaster Tycoon 2's real physics engine builds, tests, and rates them.";

pub const FONTS: &str = "https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@700;800&family=IBM+Plex+Mono:wght@400;600&display=swap";

pub const CSS: &str = include_str!("../static/site.css");
pub const JS: &str = include_str!("../static/site.js");

pub const MODE_TAGLINES: [(&str, &str); 2] = [
    ("design", "design mode — models design from scratch"),
    (
        "library",
        "library mode — models may search the stock track design library; tests retrieval and adaptation, copies score zero",
    ),
];

pub fn mode_tagline(mode: &str) -> String {
    MODE_TAGLINES
        .iter()
        .find(|(name, _)| *name == mode)
        .map(|(_, tagline)| (*tagline).to_string())
        .unwrap_or_else(|| mode.to_string())
}

pub fn fmt_tokens(n: f64) -> String {
    if n >= 1_000_000.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("{:.0}k", n / 1_000.0)
    } else {
        format!("{n:.0}")
    }
}

/// Page chrome: title bar, unfurl tags, layout width.
pub struct Chrome {
    pub title: String,
    pub titlebar: String,
    /// Page path relative to the site root, for og:url.
    pub path: String,
    /// Public URL the site deploys to. Slack ignores relative og:image/og:url,
    /// so those tags only appear when a base URL is known.
    pub base_url: Option<String>,
    /// Page width: prose by default, `Mid` for the leaderboard, `Wide` for
    /// the multi-column run and round grids.
    pub width: Width,
    /// Pull in mermaid (from jsDelivr) only on the page that draws a diagram.
    pub needs_mermaid: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Prose,
    Mid,
    Wide,
}

impl Chrome {
    pub fn new(title: &str, titlebar: &str, path: &str, base_url: Option<&str>) -> Self {
        Self {
            title: title.to_string(),
            titlebar: titlebar.to_string(),
            path: path.to_string(),
            base_url: base_url.map(|b| b.trim_end_matches('/').to_string()),
            width: Width::Prose,
            needs_mermaid: false,
        }
    }

    pub fn width(mut self, width: Width) -> Self {
        self.width = width;
        self
    }

    pub fn with_mermaid(mut self) -> Self {
        self.needs_mermaid = true;
        self
    }

    pub fn tagline(&self) -> &'static str {
        TAGLINE
    }

    pub fn css(&self) -> &'static str {
        CSS
    }

    pub fn js(&self) -> &'static str {
        JS
    }

    pub fn fonts(&self) -> &'static str {
        FONTS
    }

    pub fn wrap_class(&self) -> &'static str {
        match self.width {
            Width::Prose => "wrap",
            Width::Mid => "wrap wrap-mid",
            Width::Wide => "wrap wrap-wide",
        }
    }

    pub fn og_url(&self) -> Option<String> {
        self.base_url
            .as_ref()
            .map(|base| format!("{base}/{}", self.path))
    }

    pub fn og_image(&self) -> Option<String> {
        self.base_url
            .as_ref()
            .map(|base| format!("{base}/og-card.png"))
    }
}

/// One preview tile in a track design gallery.
pub struct Figure {
    pub name: String,
    pub caption: String,
    pub src: Option<String>,
}

/// One row of the index table: a single model's collection of rounds within a
/// run (runs are pivoted, so a three-model run is three rows).
pub struct IndexRow {
    pub run_name: String,
    pub run_href: String,
    /// The model's own detail page.
    pub model_href: String,
    pub date: String,
    pub mode: String,
    pub coaster: String,
    pub harness: String,
    pub model: String,
    pub thumb: Option<String>,
    pub place: String,
    pub is_winner: bool,
    /// True on the first row of each run, which draws the group separator.
    pub starts_run: bool,
    pub score: Option<String>,
    pub intensity: String,
    pub nausea: String,
    pub best_round: String,
    pub usage: String,
}

pub struct Facet {
    pub name: String,
    pub values: Vec<String>,
}

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexPage {
    pub chrome: Chrome,
    pub facets: Vec<Facet>,
    pub rows: Vec<IndexRow>,
    pub have_previews: bool,
    pub mode_taglines: Vec<(String, String)>,
    /// Runs left out because they never finished, newest first.
    pub skipped: Vec<String>,
}

pub struct StandingRow {
    pub place: String,
    pub model: String,
    pub is_winner: bool,
    pub score: Option<String>,
    pub intensity: String,
    pub nausea: String,
    pub similarity: String,
    pub best_round: String,
    pub usage: String,
}

pub struct Shot {
    pub src: String,
    pub label: String,
}

pub struct RoundView {
    pub number: u32,
    pub build_error: Option<String>,
    /// Rating line, absent when the round produced no rated ride.
    pub stats: Option<RoundStats>,
    pub unrated_note: bool,
    pub lookups: Option<String>,
    pub program_json: Option<String>,
    pub program_pieces: usize,
    pub shots: Vec<Shot>,
    /// The rotator's shot list / labels as JSON, for the client-side flipper.
    pub shots_json: String,
    pub labels_json: String,
}

pub struct RoundStats {
    pub excitement: String,
    pub intensity: String,
    pub nausea: String,
    pub ride_length: i64,
    pub num_drops: i64,
    pub total_air_time: i64,
    pub crashed: bool,
    pub similarity: Option<String>,
    /// Set when the copy penalty actually cut the score.
    pub penalized: Option<String>,
}

pub struct ModelView {
    pub model: String,
    /// This model's detail page, linked from the run comparison.
    pub href: String,
    pub chart_svg: String,
    pub studied: Vec<Figure>,
    pub rounds: Vec<RoundView>,
}

#[derive(Template)]
#[template(path = "run.html")]
pub struct RunPage {
    pub chrome: Chrome,
    pub mode_tagline: String,
    pub grace: String,
    pub standings: Vec<StandingRow>,
    pub models: Vec<ModelView>,
}

/// A headline number on the model detail page.
pub struct Stat {
    pub label: String,
    pub value: String,
    /// CSS class for the value, e.g. "rating-excitement".
    pub class: String,
}

/// One model's whole run: the detail view an index row links to.
#[derive(Template)]
#[template(path = "model.html")]
pub struct ModelPage<'a> {
    pub chrome: Chrome,
    pub run_name: String,
    pub run_href: String,
    pub place: String,
    pub of_models: usize,
    pub context: String,
    pub stats: Vec<Stat>,
    pub model: &'a ModelView,
}

#[derive(Template)]
#[template(path = "library.html")]
pub struct LibraryPage {
    pub chrome: Chrome,
    pub figures: Vec<Figure>,
}
