//! The smokeping-style latency heatmap: hours x days, colour = how slow that
//! hour was *relative to the monitor's own median*. Patterns pop out at a
//! glance ("slow every Monday at 9am") without any threshold to tune - the
//! info-only companion of an adaptive baseline, with zero false-positive risk.

use std::collections::HashMap;
use std::fmt::Write as _;

use chrono::DateTime;

use hora_core::fmt::xml_escape;

/// The window rendered: four full weeks, so each weekday appears four times
/// and a weekly pattern is visible as a horizontal stripe rhythm.
pub(crate) const HEATMAP_DAYS: i64 = 28;

const CELL_W: f64 = 16.0;
const CELL_H: f64 = 9.0;
const GAP: f64 = 1.5;
/// Left gutter for the hour labels, top gutter for the day labels.
const LEFT: f64 = 34.0;
const TOP: f64 = 18.0;

/// The tier of a cell at `ratio` = cell latency / monitor median, `t0` to
/// `t4`. The tiers are deliberately coarse: the heatmap shows *patterns*,
/// not numbers (the numbers are in each cell's tooltip).
fn tier(ratio: f64) -> usize {
    if ratio <= 1.25 {
        0 // around the median: usual
    } else if ratio <= 2.0 {
        1
    } else if ratio <= 3.0 {
        2
    } else if ratio <= 5.0 {
        3
    } else {
        4 // 5x the median or worse
    }
}

/// Each tier's colour, from the brand tokens (light theme): the fill
/// attribute every viewer can read, even one that drops the stylesheet. On
/// the monitor page the stylesheet repaints the tiers from the live tokens
/// (and so follows the theme); the standalone image restyles itself for dark
/// mode with its own `<style>`.
const TIER_LIGHT: [&str; 5] = ["#d8e4e2", "#92b7b9", "#4e8a92", "#b4633f", "#a5372a"];
const TIER_DARK: [&str; 5] = ["#1d2a31", "#2d525a", "#3e7a84", "#d89372", "#e07a63"];

/// The standalone image's own theme: a dark variant for dark-mode viewers.
fn standalone_style() -> String {
    let mut style = String::from(
        "<style>@media (prefers-color-scheme: dark){.bg{fill:#1a2029}text{fill:#a8aeb8}",
    );
    for (index, color) in TIER_DARK.iter().enumerate() {
        let _ = write!(style, ".t{index}{{fill:{color}}}");
    }
    style.push_str("}</style>");
    style
}

/// Render the heatmap SVG from `(hour_ts, avg_latency_ms)` cells (as returned
/// by [`hora_core::db::latency_hourly`]). `now` anchors the window: the last
/// column is today (UTC), hours run top to bottom. `standalone` is the image
/// served on its own (it carries its dark-mode style); the page inlines it
/// without, the stylesheet doing that job.
pub(crate) fn render(
    cells: &[(i64, i64)],
    now: i64,
    monitor_name: &str,
    standalone: bool,
) -> String {
    let day_start = (now / 86_400 - (HEATMAP_DAYS - 1)) * 86_400;
    let by_hour: HashMap<i64, i64> = cells.iter().copied().collect();

    // The reference for "how slow is unusual": the median of the rendered
    // cells. A monitor with a stable baseline shows green everywhere; the
    // outliers carry the colour.
    let mut latencies: Vec<i64> = cells
        .iter()
        .filter(|(hour, _)| *hour >= day_start)
        .map(|(_, latency)| *latency)
        .collect();
    latencies.sort_unstable();
    let median = latencies
        .get(latencies.len() / 2)
        .copied()
        .unwrap_or(1)
        .max(1);

    let width = LEFT + coordf(HEATMAP_DAYS) * CELL_W + GAP;
    let height = TOP + 24.0 * CELL_H + GAP;
    let name = xml_escape(monitor_name);
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {width:.0} {height:.0}\" \
         width=\"{width:.0}\" height=\"{height:.0}\" role=\"img\" \
         aria-label=\"Latency heatmap of {name}, last {HEATMAP_DAYS} days\" \
         font-family=\"ui-sans-serif,system-ui,sans-serif\" font-size=\"8\">"
    );
    if standalone {
        svg.push_str(&standalone_style());
    }
    let _ = write!(
        svg,
        "<rect class=\"bg\" width=\"{width:.0}\" height=\"{height:.0}\" rx=\"6\" fill=\"#faf9f5\"/>"
    );

    // Hour gutter: a label every six rows is enough to orient.
    for hour in [0_i64, 6, 12, 18] {
        let y = TOP + coordf(hour) * CELL_H + 7.0;
        let _ = write!(
            svg,
            "<text x=\"{x:.0}\" y=\"{y:.1}\" fill=\"#525a66\" text-anchor=\"end\">{hour:02}h</text>",
            x = LEFT - 5.0,
        );
    }

    for day in 0..HEATMAP_DAYS {
        let day_ts = day_start + day * 86_400;
        let x = LEFT + coordf(day) * CELL_W;

        // Day labels weekly, anchored on the column's weekday ("Mon 08"):
        // the weekday is the whole point of a 4-week window.
        if day % 7 == 0
            && let Some(date) = DateTime::from_timestamp(day_ts, 0)
        {
            let _ = write!(
                svg,
                "<text x=\"{x:.1}\" y=\"{y:.0}\" fill=\"#525a66\">{label}</text>",
                y = TOP - 6.0,
                label = date.format("%a %d"),
            );
        }

        for hour in 0..24_i64 {
            let Some(latency) = by_hour.get(&(day_ts + hour * 3600)) else {
                continue; // no data: the background shows through
            };
            let ratio = coordf(*latency) / coordf(median);
            let tier = tier(ratio);
            let color = TIER_LIGHT[tier];
            let y = TOP + coordf(hour) * CELL_H;
            let title = DateTime::from_timestamp(day_ts, 0).map_or_else(String::new, |date| {
                format!(
                    "{} {:02}:00 UTC - {latency} ms",
                    date.format("%a %Y-%m-%d"),
                    hour
                )
            });
            let _ = write!(
                svg,
                "<rect class=\"t{tier}\" x=\"{x:.1}\" y=\"{y:.1}\" width=\"{w:.1}\" height=\"{h:.1}\" \
                 rx=\"1.5\" fill=\"{color}\"><title>{title}</title></rect>",
                w = CELL_W - GAP,
                h = CELL_H - GAP,
            );
        }
    }

    svg.push_str("</svg>");
    svg
}

/// `i64` to SVG coordinate, mirroring `render::coord` for the i64-heavy math here.
fn coordf(value: i64) -> f64 {
    crate::render::coord(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_scale_with_the_ratio() {
        assert_eq!(tier(1.0), 0);
        assert_eq!(tier(1.8), 1);
        assert_eq!(tier(2.5), 2);
        assert_eq!(tier(4.0), 3);
        assert_eq!(tier(10.0), 4);
    }

    #[test]
    fn renders_cells_relative_to_the_median() {
        let now = 30 * 86_400; // some UTC midnight
        // Three cells in-window: two at the median (100ms), one 10x slower.
        let day = (now / 86_400 - 1) * 86_400;
        let cells = vec![(day, 100), (day + 3600, 100), (day + 7200, 1000)];

        let svg = render(&cells, now, "API <prod>", true);
        assert!(svg.starts_with("<svg"));
        // The name is escaped into the aria-label.
        assert!(svg.contains("API &lt;prod&gt;"));
        // Median cells are the usual tier, the outlier the worst; the fills
        // are the brand's (up is petrol, never green); tooltips carry numbers.
        assert!(svg.contains("class=\"t0\"") && svg.contains(TIER_LIGHT[0]));
        assert!(svg.contains("class=\"t4\"") && svg.contains(TIER_LIGHT[4]));
        assert!(!svg.contains("#10b981"));
        assert!(svg.contains("1000 ms"));
        // Exactly three data cells are drawn (plus the background rect).
        assert_eq!(svg.matches("<rect").count(), 4);
        // The standalone image follows dark mode by itself; the inline one
        // carries no <style> (the page's CSP has no 'unsafe-inline').
        assert!(svg.contains("prefers-color-scheme: dark"));
        assert!(!render(&cells, now, "API", false).contains("<style"));
    }

    #[test]
    fn empty_series_renders_background_only() {
        let svg = render(&[], 30 * 86_400, "API", false);
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
        assert_eq!(svg.matches("<rect").count(), 1);
    }
}
