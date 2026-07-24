//! Inline SVG of excitement/intensity per round, failed builds marked.
//!
//! Hand-rolled rather than pulled from a plotting crate: it is one chart type,
//! it has to inherit the page's CSS variables, and the whole thing is ~60
//! lines of coordinate math.

use std::fmt::Write as _;

use crate::model::ModelRun;

const WIDTH: f64 = 640.0;
const HEIGHT: f64 = 200.0;
const LEFT: f64 = 36.0;
const RIGHT: f64 = 12.0;
const TOP: f64 = 16.0;
const BOTTOM: f64 = 26.0;
/// Ratings top out well below this; a fixed ceiling keeps every model's chart
/// on the same scale so they can be compared across columns.
const Y_MAX: f64 = 16.0;

fn escape(text: &str) -> String {
    askama::filters::escape(text, askama::filters::Html)
        .map(|e| e.to_string())
        .unwrap_or_default()
}

/// Returns the chart markup, or an empty string for models with fewer than
/// two rounds (a single point is not a trend).
pub fn round_chart(model: &ModelRun) -> String {
    let rounds = &model.rounds;
    if rounds.len() < 2 {
        return String::new();
    }
    let plot_w = WIDTH - LEFT - RIGHT;
    let plot_h = HEIGHT - TOP - BOTTOM;
    let last = (rounds.len() - 1).max(1) as f64;
    let x_of = |i: usize| LEFT + plot_w * (i as f64 / last);
    let y_of = |value: f64| TOP + plot_h * (1.0 - value.min(Y_MAX) / Y_MAX);

    let mut svg = String::new();
    for gy in [0.0, 5.0, 15.0] {
        let y = y_of(gy);
        let _ = write!(
            svg,
            r#"<line x1="{LEFT}" y1="{y:.1}" x2="{}" y2="{y:.1}" stroke="var(--panel-deep)" stroke-width="1"/>"#,
            WIDTH - RIGHT
        );
        let _ = write!(
            svg,
            r#"<text x="{}" y="{:.1}" text-anchor="end" font-size="10" fill="var(--ink-soft)">{gy:.0}</text>"#,
            LEFT - 6.0,
            y + 4.0
        );
    }
    // The intensity ceiling is the story; draw it as a labeled dashed line.
    let ceiling = y_of(10.0);
    let _ = write!(
        svg,
        r#"<line x1="{LEFT}" y1="{ceiling:.1}" x2="{}" y2="{ceiling:.1}" stroke="var(--ink-soft)" stroke-width="1" stroke-dasharray="5 4"/>"#,
        WIDTH - RIGHT
    );
    let _ = write!(
        svg,
        r#"<text x="{}" y="{:.1}" text-anchor="end" font-size="10" fill="var(--ink-soft)">intensity ceiling 10</text>"#,
        WIDTH - RIGHT,
        ceiling - 5.0
    );

    let best_round = model.best().map(|r| r.number);
    for (name, colour, values) in [
        (
            "excitement",
            "var(--excitement-mark)",
            rounds
                .iter()
                .map(|r| r.ride.as_ref().and_then(|ride| ride.excitement))
                .collect::<Vec<_>>(),
        ),
        (
            "intensity",
            "var(--intensity-mark)",
            rounds
                .iter()
                .map(|r| r.ride.as_ref().and_then(|ride| ride.intensity))
                .collect::<Vec<_>>(),
        ),
    ] {
        // Rounds that never got rated break the line rather than interpolating
        // across a gap that did not happen.
        let mut segment: Vec<String> = Vec::new();
        let flush = |segment: &mut Vec<String>, svg: &mut String| {
            if segment.len() > 1 {
                let _ = write!(
                    svg,
                    r#"<polyline points="{}" fill="none" stroke="{colour}" stroke-width="2"/>"#,
                    segment.join(" ")
                );
            }
            segment.clear();
        };
        for (i, value) in values.iter().enumerate() {
            match value {
                Some(v) => segment.push(format!("{:.1},{:.1}", x_of(i), y_of(*v))),
                None => flush(&mut segment, &mut svg),
            }
        }
        flush(&mut segment, &mut svg);

        for (i, value) in values.iter().enumerate() {
            let Some(value) = value else { continue };
            let (cx, cy) = (x_of(i), y_of(*value));
            let round = &rounds[i];
            let title = format!("<title>round {}: {name} {value:.2}</title>", round.number);
            if name == "excitement" {
                let _ = write!(
                    svg,
                    r#"<circle cx="{cx:.1}" cy="{cy:.1}" r="5" fill="{colour}" stroke="var(--panel-dark)" stroke-width="2">{title}</circle>"#
                );
                if best_round == Some(round.number) {
                    let _ = write!(
                        svg,
                        r#"<text x="{:.1}" y="{:.1}" text-anchor="end" font-size="11" font-weight="600" fill="var(--ink)">{value:.2}</text>"#,
                        cx - 9.0,
                        cy - 9.0
                    );
                }
            } else {
                let _ = write!(
                    svg,
                    r#"<rect x="{:.1}" y="{:.1}" width="8" height="8" fill="{colour}" stroke="var(--panel-dark)" stroke-width="2">{title}</rect>"#,
                    cx - 4.0,
                    cy - 4.0
                );
            }
        }
    }

    let baseline = y_of(0.0);
    for (i, round) in rounds.iter().enumerate() {
        let cx = x_of(i);
        if let Some(error) = &round.build_error {
            let _ = write!(
                svg,
                r#"<text x="{cx:.1}" y="{:.1}" text-anchor="middle" font-size="13" font-weight="600" fill="var(--fail)">&#215;<title>round {}: {}</title></text>"#,
                baseline + 4.0,
                round.number,
                escape(error)
            );
        } else if round.ride.is_none() {
            let _ = write!(
                svg,
                r#"<circle cx="{cx:.1}" cy="{baseline:.1}" r="4" fill="none" stroke="var(--ink-soft)" stroke-width="2"><title>round {}: built but not rated (test never completed)</title></circle>"#,
                round.number
            );
        }
        let _ = write!(
            svg,
            r#"<text x="{cx:.1}" y="{}" text-anchor="middle" font-size="10" fill="var(--ink-soft)">R{}</text>"#,
            HEIGHT - 8.0,
            round.number
        );
    }

    let legend = concat!(
        r#"<div class="chart-legend">"#,
        r#"<span><span class="chip" style="background:var(--excitement-mark);border-radius:50%"></span>excitement</span>"#,
        r#"<span><span class="chip" style="background:var(--intensity-mark)"></span>intensity</span>"#,
        r#"<span><span class="chip" style="background:none;color:var(--fail)">&#215;</span>build failed</span>"#,
        "</div>",
    );
    format!(
        r#"<div class="chart">{legend}<svg viewBox="0 0 {WIDTH:.0} {HEIGHT:.0}" role="img" aria-label="excitement and intensity per round for {model}">{svg}</svg></div>"#,
        model = escape(&model.model)
    )
}
