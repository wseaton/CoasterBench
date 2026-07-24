//! Static site generator for CoasterBench eval runs.
//!
//! Renders evals/runs/ into a self-contained static site styled after the
//! OpenRCT2 in-game window chrome: an index leaderboard (one row per model per
//! run) plus one page per run with every model's rounds, ratings, track
//! programs, and park screenshots.

mod art;
mod chart;
mod images;
mod model;
mod view;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use askama::Template;
use clap::Parser;

use art::{Art, ArtStore};
use model::{EvalRun, ModelRun, Round};
use view::{
    Badge, Chrome, Facet, Figure, IndexPage, IndexRow, LibraryPage, ModelPage, ModelView,
    RoundStats, RoundView, RunPage, Shot, StandingRow, Stat,
};

#[derive(Parser)]
#[command(about, long_about = None)]
struct Args {
    /// Runs directory.
    #[arg(long, default_value = "evals/runs")]
    runs: PathBuf,
    /// Output directory.
    #[arg(long, default_value = "evals/site")]
    out: PathBuf,
    /// Rendered track design previews.
    #[arg(long, default_value = "evals/library-previews")]
    previews: PathBuf,
    /// Manifest of previews uploaded to the artifact store.
    #[arg(long, default_value = "evals/library-previews.json")]
    previews_manifest: PathBuf,
    /// Public URL the site deploys to; required for Slack unfurl images,
    /// which must be absolute URLs. Pass an empty string to omit them.
    #[arg(long, default_value = "https://wseaton.github.io/CoasterBench")]
    base_url: String,
    /// Public URL of the R2 artifact store holding screenshots not present
    /// locally (see evals/publish.py).
    #[arg(long, default_value = "https://artifacts.wseaton.com")]
    artifact_base: String,
    /// Render runs that never finished (some model short of its rounds).
    #[arg(long)]
    include_partial: bool,
}

fn place_label(place: usize) -> String {
    match place {
        1 => "1st".to_string(),
        2 => "2nd".to_string(),
        3 => "3rd".to_string(),
        other => format!("{other}th"),
    }
}

fn usage_cell(model: &ModelRun) -> String {
    let totals = model.usage_totals();
    let mut parts = Vec::new();
    if totals.tokens_in > 0.0 || totals.tokens_out > 0.0 {
        parts.push(format!(
            "{} in / {} out",
            view::fmt_tokens(totals.tokens_in),
            view::fmt_tokens(totals.tokens_out)
        ));
    }
    if let Some(cost) = totals.cost_usd {
        parts.push(format!("${cost:.2}"));
    }
    if parts.is_empty() {
        "—".to_string()
    } else {
        parts.join(" · ")
    }
}

fn fmt_opt(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_string(), |v| format!("{v:.2}"))
}

/// Copies a local artifact into the site and returns its src, or hands back
/// the artifact store URL for remote-only ones.
fn place_art(art: &Art, out: &Path, rel: &Path) -> Result<Option<String>> {
    let Some(local) = &art.local else {
        return Ok(art.url.clone());
    };
    let dest = out.join(rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(local, &dest)
        .with_context(|| format!("copying {} to {}", local.display(), dest.display()))?;
    Ok(Some(rel.to_string_lossy().replace('\\', "/")))
}

/// Preview tiles for a list of (design name, caption) pairs.
fn figures(designs: &[(String, String)], store: &ArtStore, out: &Path) -> Result<Vec<Figure>> {
    let mut figures = Vec::with_capacity(designs.len());
    for (name, caption) in designs {
        let src = match store.preview(name) {
            Some(art) => {
                let rel = Path::new("assets").join("library").join(&art.name);
                place_art(&art, out, &rel)?
            }
            None => None,
        };
        figures.push(Figure {
            name: name.clone(),
            caption: caption.clone(),
            src,
        });
    }
    Ok(figures)
}

fn round_stats(round: &Round) -> Option<RoundStats> {
    let ride = round.ride.as_ref()?;
    let excitement = ride.excitement?;
    let similarity = round.similarity.as_ref().map(|sim| {
        format!(
            "similarity {:.2} (nearest: {})",
            sim.similarity, sim.nearest_design
        )
    });
    let penalized = (round.excitement() < excitement)
        .then(|| format!("penalized score {:.2}", round.excitement()));
    Some(RoundStats {
        excitement: format!("{excitement:.2}"),
        intensity: fmt_opt(ride.intensity),
        nausea: fmt_opt(ride.nausea),
        ride_length: ride.ride_length,
        num_drops: ride.num_drops,
        total_air_time: ride.total_air_time,
        crashed: ride.crashed,
        similarity,
        penalized,
    })
}

/// The chip beside a round heading. Only outcomes that need explaining get
/// one; a round that built, tested and rated cleanly says nothing.
fn round_badge(round: &Round, stats: Option<&RoundStats>) -> Option<Badge> {
    let badge = |text: &str, class: &str| {
        Some(Badge {
            text: text.to_string(),
            class: class.to_string(),
        })
    };
    if round.build_error.is_some() {
        return badge("build failed", "badge-fail");
    }
    match stats {
        Some(stats) if stats.crashed => badge("crashed", "badge-fail"),
        Some(_) => None,
        None => badge("not rated", "badge-warn"),
    }
}

fn lookups_line(round: &Round) -> Option<String> {
    if round.lookups.is_empty() {
        return None;
    }
    let searches = round.lookups.iter().filter(|l| l.tool == "search").count();
    let mut line = if searches > 0 {
        format!("library: {searches} search(es)")
    } else {
        "library:".to_string()
    };
    let studied = round.fetched_designs();
    if !studied.is_empty() {
        line.push_str(" studied ");
        line.push_str(&studied.join(", "));
    }
    Some(line)
}

/// Every capture of a round, in rotator order: the main view, extra rotations,
/// then the x-ray.
fn round_shots(round: &Round, out: &Path, run: &EvalRun, model: &str) -> Result<Vec<Shot>> {
    let mut arts: Vec<(&Art, String)> = Vec::new();
    if let Some(shot) = &round.screenshot {
        arts.push((shot, "view 1".to_string()));
    }
    for (i, shot) in round.rotation_shots.iter().enumerate() {
        arts.push((shot, format!("view {}", i + 2)));
    }
    if let Some(shot) = &round.xray_shot {
        arts.push((shot, "x-ray".to_string()));
    }
    let mut shots = Vec::with_capacity(arts.len());
    for (art, label) in arts {
        let rel = Path::new("assets")
            .join(&run.name)
            .join(model)
            .join(format!("round_{}_{}", round.number, art.name));
        if let Some(src) = place_art(art, out, &rel)? {
            shots.push(Shot { src, label });
        }
    }
    Ok(shots)
}

fn standings(run: &EvalRun) -> Vec<StandingRow> {
    run.ranked()
        .iter()
        .enumerate()
        .map(|(i, model)| {
            let best = model.best();
            let ride = best.and_then(|r| r.ride.as_ref());
            StandingRow {
                place: place_label(i + 1),
                model: model.model.clone(),
                is_winner: i == 0 && best.is_some(),
                score: best.map(|r| format!("{:.2}", r.excitement())),
                intensity: fmt_opt(ride.and_then(|r| r.intensity)),
                nausea: fmt_opt(ride.and_then(|r| r.nausea)),
                similarity: fmt_opt(
                    best.and_then(|r| r.similarity.as_ref())
                        .map(|s| s.similarity),
                ),
                best_round: best.map_or_else(String::new, |r| {
                    format!("round {}/{}", r.number, model.rounds.len())
                }),
                usage: usage_cell(model),
            }
        })
        .collect()
}

/// The page a run's model links to: `run-<run>-<model>.html`.
fn model_href(run: &EvalRun, model: &str) -> String {
    format!("run-{}-{}.html", run.name, model::sanitise_name(model))
}

/// One round's session trace lives on its own page: a long round runs to
/// hundreds of events, which would bury the ratings on the model page.
fn trace_href(run: &EvalRun, model: &str, round: u32) -> String {
    format!(
        "trace-{}-{}-r{round}.html",
        run.name,
        model::sanitise_name(model)
    )
}

fn fmt_elapsed(ms: i64) -> String {
    let secs = ms.max(0) / 1000;
    format!("{}:{:02}", secs / 60, secs % 60)
}

fn fmt_json_compact(value: &serde_json::Value, limit: usize) -> Option<String> {
    let text = match value {
        serde_json::Value::Null => return None,
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let text = text.trim();
    if text.is_empty() || text == "{}" {
        return None;
    }
    Some(clamp(text, limit))
}

/// Long tool payloads are truncated: a `valid_next_pieces` result runs to
/// kilobytes and the interesting part is always at the front.
fn clamp(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let head: String = text.chars().take(limit).collect();
    format!("{head}… ({} chars)", text.chars().count())
}

fn build_trace_page(
    run: &EvalRun,
    model: &ModelRun,
    round: &Round,
    base_url: Option<&str>,
    out: &Path,
) -> Result<()> {
    let tools = round.trace.iter().filter(|e| e.is_tool()).count();
    let rejected = round.trace.iter().filter(|e| e.is_rejection()).count();
    let elapsed = round.trace.iter().filter_map(|e| e.ms).max().unwrap_or(0);
    let cost: f64 = round.trace.iter().filter_map(|e| e.cost_usd).sum();

    let mut summary = vec![
        view::TraceSummary {
            label: "events".into(),
            value: round.trace.len().to_string(),
            class: String::new(),
        },
        view::TraceSummary {
            label: "tool calls".into(),
            value: tools.to_string(),
            class: String::new(),
        },
        view::TraceSummary {
            label: "rejected".into(),
            value: rejected.to_string(),
            class: if rejected > 0 { "fail" } else { "dim" }.into(),
        },
        view::TraceSummary {
            label: "session".into(),
            value: fmt_elapsed(elapsed),
            class: String::new(),
        },
    ];
    if cost > 0.0 {
        summary.push(view::TraceSummary {
            label: "cost".into(),
            value: format!("${cost:.2}"),
            class: "usage".into(),
        });
    }

    let rows = round
        .trace
        .iter()
        .map(|event| view::TraceRow {
            kind: event.kind.clone(),
            at: event.ms.map(fmt_elapsed).unwrap_or_default(),
            text: event.text.as_ref().map(|t| clamp(t, 4000)),
            tool: event.name.clone(),
            input: event.input.as_ref().and_then(|v| fmt_json_compact(v, 1200)),
            output: event
                .output
                .as_ref()
                .and_then(|v| fmt_json_compact(v, 1200)),
            failed: event.is_rejection(),
            duration: event.dur_ms.map(|d| format!("{d} ms")),
            cost: event
                .cost_usd
                .filter(|c| *c > 0.0)
                .map(|c| format!("${c:.4}")),
        })
        .collect();

    let path = trace_href(run, &model.model, round.number);
    let page = view::TracePage {
        chrome: Chrome::new(
            &format!(
                "Coaster Evals — {} · {} · round {}",
                run.name, model.model, round.number
            ),
            &format!("TRACE · ROUND {}", round.number),
            &path,
            base_url,
        )
        .width(view::Width::Mid),
        run_name: run.name.clone(),
        run_href: format!("run-{}.html", run.name),
        model: model.model.clone(),
        model_href: model_href(run, &model.model),
        round: round.number,
        summary,
        rows,
    };
    write_page(&out.join(&path), &page.render()?)
}

/// Builds one model's view (chart, studied designs, every round). Copies the
/// round captures into the site as a side effect, so it runs once per model
/// and both the run comparison and the model detail page render from it.
fn build_model_view(
    run: &EvalRun,
    model: &ModelRun,
    store: &ArtStore,
    out: &Path,
) -> Result<ModelView> {
    let studied: Vec<(String, String)> = model
        .studied_designs()
        .into_iter()
        .map(|name| (name.clone(), name))
        .collect();
    let mut rounds = Vec::with_capacity(model.rounds.len());
    for round in &model.rounds {
        let shots = round_shots(round, out, run, &model.model)?;
        let stats = round_stats(round);
        rounds.push(RoundView {
            number: round.number,
            trace_href: (!round.trace.is_empty())
                .then(|| trace_href(run, &model.model, round.number)),
            trace_events: round.trace.len(),
            trace_rejections: round.trace.iter().filter(|e| e.is_rejection()).count(),
            badge: round_badge(round, stats.as_ref()),
            build_error: round.build_error.clone(),
            unrated_note: stats.is_none() && round.build_error.is_none(),
            stats,
            lookups: lookups_line(round),
            program_json: round.program_json.clone(),
            program_pieces: round.program_pieces,
            shots_json: serde_json::to_string(&shots.iter().map(|s| &s.src).collect::<Vec<_>>())?,
            labels_json: serde_json::to_string(
                &shots.iter().map(|s| &s.label).collect::<Vec<_>>(),
            )?,
            shots,
        });
    }
    Ok(ModelView {
        model: model.model.clone(),
        href: model_href(run, &model.model),
        chart_svg: chart::round_chart(model),
        studied: figures(&studied, store, out)?,
        rounds,
    })
}

/// The headline numbers at the top of a model detail page.
fn model_stats(model: &ModelRun) -> Vec<Stat> {
    let stat = |label: &str, value: String, class: &str| Stat {
        label: label.to_string(),
        value,
        class: class.to_string(),
    };
    let best = model.best();
    let ride = best.and_then(|r| r.ride.as_ref());
    let rated = model.rounds.iter().filter(|r| r.excitement() > 0.0).count();
    let mut stats = vec![
        stat(
            "score",
            best.map_or_else(|| "—".to_string(), |r| format!("{:.2}", r.excitement())),
            "rating-excitement",
        ),
        stat(
            "intensity",
            fmt_opt(ride.and_then(|r| r.intensity)),
            "rating-intensity",
        ),
        stat(
            "nausea",
            fmt_opt(ride.and_then(|r| r.nausea)),
            "rating-nausea",
        ),
        stat(
            "best round",
            best.map_or_else(|| "—".to_string(), |r| format!("{}", r.number)),
            "",
        ),
        stat(
            "rated rounds",
            format!("{rated}/{}", model.rounds.len()),
            "",
        ),
    ];
    if let Some(sim) = best.and_then(|r| r.similarity.as_ref()) {
        stats.push(stat("similarity", format!("{:.2}", sim.similarity), "dim"));
    }
    stats.push(stat("tokens / cost", usage_cell(model), "usage"));
    stats
}

fn build_model_page(
    run: &EvalRun,
    view: &ModelView,
    place: usize,
    base_url: Option<&str>,
    out: &Path,
) -> Result<()> {
    let model = run
        .models
        .iter()
        .find(|m| m.model == view.model)
        .context("model view without a model")?;
    let path = model_href(run, &view.model);
    let page = ModelPage {
        chrome: Chrome::new(
            &format!("Coaster Evals — {} · {}", run.name, view.model),
            &format!("{} · {}", view.model.to_uppercase(), run.name),
            &path,
            base_url,
        )
        .width(view::Width::Wide),
        run_name: run.name.clone(),
        run_href: format!("run-{}.html", run.name),
        place: place_label(place),
        of_models: run.models.len(),
        context: format!(
            "{} · {} · {} · {}",
            run.mode,
            run.ride_name(),
            run.harness,
            view::mode_tagline(&run.mode)
        ),
        stats: model_stats(model),
        model: view,
    };
    write_page(&out.join(&path), &page.render()?)
}

fn build_run_page(
    run: &EvalRun,
    store: &ArtStore,
    out: &Path,
    base_url: Option<&str>,
) -> Result<()> {
    let mut models = Vec::with_capacity(run.models.len());
    for model in run.ranked() {
        models.push(build_model_view(run, model, store, out)?);
    }
    for (i, view) in models.iter().enumerate() {
        build_model_page(run, view, i + 1, base_url, out)?;
    }
    for model in &run.models {
        for round in model.rounds.iter().filter(|r| !r.trace.is_empty()) {
            build_trace_page(run, model, round, base_url, out)?;
        }
    }

    let path = format!("run-{}.html", run.name);
    let page = RunPage {
        chrome: Chrome::new(
            &format!("Coaster Evals — {}", run.name),
            &format!("RUN {} ({})", run.name, run.mode),
            &path,
            base_url,
        )
        .width(view::Width::Wide),
        mode_tagline: view::mode_tagline(&run.mode),
        grace: format!("{}", run.grace),
        standings: standings(run),
        models,
    };
    write_page(&out.join(&path), &page.render()?)
}

fn write_page(path: &Path, html: &str) -> Result<()> {
    std::fs::write(path, html).with_context(|| format!("writing {}", path.display()))
}

fn thumbnail(
    shot: &Art,
    store: &ArtStore,
    out: &Path,
    run: &EvalRun,
    model: &str,
) -> Result<Option<String>> {
    let Some(src) = store.pixels(shot) else {
        return Ok(None);
    };
    let rel = Path::new("assets").join("thumbs").join(format!(
        "{}-{}.png",
        run.name,
        model::sanitise_name(model)
    ));
    images::write_thumbnail(&src, &out.join(&rel))?;
    Ok(Some(rel.to_string_lossy().replace('\\', "/")))
}

/// Index rows: one per model per run, best score first (the leaderboard
/// question is "who built the best coaster", not "what ran most recently").
/// The client-side sorter can reorder by any column from here.
fn index_rows(runs: &[EvalRun], store: &ArtStore, out: &Path) -> Result<Vec<IndexRow>> {
    let mut rows = Vec::new();
    for run in runs {
        for (i, model) in run.ranked().iter().enumerate() {
            let best = model.best();
            let ride = best.and_then(|r| r.ride.as_ref());
            let shot = best
                .and_then(|r| r.screenshot.as_ref())
                .or_else(|| model.rounds.iter().find_map(|r| r.screenshot.as_ref()));
            let thumb = match shot {
                Some(shot) => thumbnail(shot, store, out, run, &model.model)?,
                None => None,
            };
            rows.push(IndexRow {
                run_name: run.name.clone(),
                run_href: format!("run-{}.html", run.name),
                model_href: model_href(run, &model.model),
                date: run.date(),
                mode: run.mode.clone(),
                coaster: run.ride_name(),
                harness: run.harness.clone(),
                model: model.model.clone(),
                thumb,
                place: place_label(i + 1),
                is_winner: i == 0 && best.is_some(),
                sort_score: best.map_or(0.0, |r| r.excitement()),
                sort_intensity: ride.and_then(|r| r.intensity).unwrap_or(0.0),
                sort_nausea: ride.and_then(|r| r.nausea).unwrap_or(0.0),
                score: best.map(|r| format!("{:.2}", r.excitement())),
                intensity: fmt_opt(ride.and_then(|r| r.intensity)),
                nausea: fmt_opt(ride.and_then(|r| r.nausea)),
                best_round: best.map_or_else(String::new, |r| {
                    format!("round {}/{}", r.number, model.rounds.len())
                }),
                usage: usage_cell(model),
            });
        }
    }
    rows.sort_by(|a, b| b.sort_score.total_cmp(&a.sort_score));
    Ok(rows)
}

/// Every finished model run, as head-to-head contenders. Draws from all runs,
/// not just the ones shown on the leaderboard: a model that ran its full round
/// count inside an otherwise-abandoned run is a valid opponent (that is the
/// only way "kimi vs fable" works, since fable's twister run was hidden by its
/// missing rivals). Generates each contender's thumbnail as a side effect.
fn contenders(runs: &[EvalRun], store: &ArtStore, out: &Path) -> Result<Vec<view::Contender>> {
    let mut contenders = Vec::new();
    for run in runs {
        for model in &run.models {
            if !run.model_completed(model) {
                continue;
            }
            let best = model.best();
            let ride = best.and_then(|r| r.ride.as_ref());
            let shot = best
                .and_then(|r| r.screenshot.as_ref())
                .or_else(|| model.rounds.iter().find_map(|r| r.screenshot.as_ref()));
            let thumb = match shot {
                Some(shot) => thumbnail(shot, store, out, run, &model.model)?,
                None => None,
            };
            let totals = model.usage_totals();
            let href = model_href(run, &model.model);
            contenders.push(view::Contender {
                id: href.trim_end_matches(".html").to_string(),
                href,
                run: run.name.clone(),
                date: run.date(),
                model: model.model.clone(),
                coaster: run.ride_name(),
                mode: run.mode.clone(),
                harness: run.harness.clone(),
                thumb,
                score: best.map(|r| r.excitement()),
                intensity: ride.and_then(|r| r.intensity),
                nausea: ride.and_then(|r| r.nausea),
                similarity: best
                    .and_then(|r| r.similarity.as_ref())
                    .map(|s| s.similarity),
                ride_length: ride.map(|r| r.ride_length),
                airtime: ride.map(|r| r.total_air_time),
                drops: ride.map(|r| r.num_drops),
                best_round: best.map(|r| r.number),
                rounds: model.rounds.len(),
                rated_rounds: model.rounds.iter().filter(|r| r.excitement() > 0.0).count(),
                tokens: totals.tokens_in + totals.tokens_out,
                cost: totals.cost_usd,
                round_scores: model.rounds.iter().map(|r| r.excitement()).collect(),
            });
        }
    }
    // Group by scenario then best-first, so the dropdown reads top-down within
    // each coaster and the default pairing is the tightest fight available.
    contenders.sort_by(|a, b| {
        a.coaster.cmp(&b.coaster).then(
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    Ok(contenders)
}

/// Default matchup: the top two contenders of the most-contested scenario, so
/// the page opens on a real fight rather than an empty picker. A scenario is a
/// (coaster, mode) pair, matching the picker's same-scenario rule.
fn default_pair(contenders: &[view::Contender]) -> (String, String) {
    let mut by_scenario: std::collections::HashMap<(&str, &str), Vec<&view::Contender>> =
        std::collections::HashMap::new();
    for c in contenders {
        by_scenario
            .entry((&c.coaster, &c.mode))
            .or_default()
            .push(c);
    }
    let best = by_scenario
        .values()
        .filter(|group| group.len() >= 2)
        .max_by_key(|group| group.len());
    match best {
        // Groups are already score-sorted within a scenario by contenders().
        Some(group) => (group[0].id.clone(), group[1].id.clone()),
        None => {
            let id = |i: usize| contenders.get(i).map(|c| c.id.clone()).unwrap_or_default();
            (id(0), id(1))
        }
    }
}

fn build_compare_page(
    contenders: &[view::Contender],
    base_url: Option<&str>,
    out: &Path,
) -> Result<()> {
    let (default_a, default_b) = default_pair(contenders);
    let page = view::ComparePage {
        chrome: Chrome::new(
            "Coaster Evals — Head to Head",
            "HEAD TO HEAD",
            "compare.html",
            base_url,
        )
        .width(view::Width::Mid),
        contenders_json: serde_json::to_string(contenders)?,
        default_a,
        default_b,
        count: contenders.len(),
    };
    write_page(&out.join("compare.html"), &page.render()?)
}

fn facets(runs: &[EvalRun]) -> Vec<Facet> {
    let collect = |name: &str, mut values: Vec<String>| {
        values.sort();
        values.dedup();
        Facet {
            name: name.to_string(),
            values,
        }
    };
    vec![
        collect("mode", runs.iter().map(|r| r.mode.clone()).collect()),
        collect("coaster", runs.iter().map(|r| r.ride_name()).collect()),
        collect("harness", runs.iter().map(|r| r.harness.clone()).collect()),
        collect(
            "model",
            runs.iter()
                .flat_map(|r| r.models.iter().map(|m| m.model.clone()))
                .collect(),
        ),
    ]
}

/// The best-rated screenshot across every run, for the unfurl card.
fn og_shot(runs: &[EvalRun]) -> Option<&Art> {
    runs.iter()
        .flat_map(|run| run.models.iter())
        .flat_map(|model| model.rounds.iter())
        .filter(|round| round.excitement() > 0.0)
        .filter_map(|round| Some((round.excitement(), round.screenshot.as_ref()?)))
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, art)| art)
}

fn build_library_page(
    runs_dir: &Path,
    store: &ArtStore,
    out: &Path,
    base_url: Option<&str>,
) -> Result<()> {
    let mut index = model::latest_library_index(runs_dir)?;
    index.sort_by(|a, b| (a.ride_type, &a.name).cmp(&(b.ride_type, &b.name)));
    let designs: Vec<(String, String)> = if index.is_empty() {
        // No library.json yet: caption with the (sanitised) file names.
        store
            .preview_names()
            .into_iter()
            .map(|name| (name.clone(), name))
            .collect()
    } else {
        index
            .iter()
            .map(|d| {
                let caption = format!(
                    "{} · type {} · {} pieces",
                    d.name, d.ride_type, d.piece_count
                );
                (d.name.clone(), caption)
            })
            .collect()
    };
    let page = LibraryPage {
        chrome: Chrome::new(
            "Coaster Evals — Track Design Library",
            "TRACK DESIGN LIBRARY",
            "library.html",
            base_url,
        ),
        figures: figures(&designs, store, out)?,
    };
    write_page(&out.join("library.html"), &page.render()?)
}

/// Empties the previous build: stale run pages and copied assets would
/// otherwise survive a renamed or deleted run.
fn clean_output(out: &Path) -> Result<()> {
    std::fs::create_dir_all(out)?;
    let assets = out.join("assets");
    if assets.is_dir() {
        std::fs::remove_dir_all(&assets)?;
    }
    for entry in std::fs::read_dir(out)?.flatten() {
        if entry.path().extension().is_some_and(|e| e == "html") {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let base_url = (!args.base_url.is_empty()).then_some(args.base_url.as_str());

    let store = ArtStore::new(&args.artifact_base, &args.previews, &args.previews_manifest)?;

    let all_runs = model::load_runs(&args.runs, &store)?;
    let out = &args.out;
    clean_output(out)?;

    // The head-to-head draws from every finished model run, including complete
    // solo models inside runs the leaderboard hides, so build it before the
    // partial-run filter.
    let contenders = contenders(&all_runs, &store, out)?;

    // A run that died (or is still going) half way through would show up as a
    // model losing badly; leave it out of the leaderboard and say so.
    let mut skipped = Vec::new();
    let mut runs = Vec::new();
    for run in all_runs {
        match run.incomplete_reason() {
            Some(reason) if !args.include_partial => {
                eprintln!("skipping partial run {} ({reason})", run.name);
                skipped.push(run.name.clone());
            }
            _ => runs.push(run),
        }
    }

    let index = IndexPage {
        chrome: Chrome::new("Coaster Evals", "COASTER EVALS", "index.html", base_url)
            .width(view::Width::Mid)
            .with_mermaid(),
        facets: facets(&runs),
        rows: index_rows(&runs, &store, out)?,
        have_previews: store.have_previews(),
        mode_taglines: view::MODE_TAGLINES
            .iter()
            .map(|(mode, tagline)| (mode.to_string(), tagline.to_string()))
            .collect(),
        skipped,
    };
    write_page(&out.join("index.html"), &index.render()?)?;

    for run in &runs {
        build_run_page(run, &store, out, base_url)?;
    }
    if contenders.len() >= 2 {
        build_compare_page(&contenders, base_url, out)?;
    }
    if store.have_previews() {
        build_library_page(&args.runs, &store, out, base_url)?;
    }

    images::write_favicon(out)?;
    if let Some(shot) = og_shot(&runs).and_then(|art| store.pixels(art)) {
        images::write_og_card(&shot, out)?;
        println!("wrote og-card.png to {}", out.display());
    }
    if base_url.is_none() {
        eprintln!(
            "note: no --base-url given, og:image/og:url omitted (Slack unfurls will be text-only)"
        );
    }
    println!(
        "wrote {} page(s) for {} run(s) to {}",
        1 + runs.len(),
        runs.len(),
        out.display()
    );
    Ok(())
}
