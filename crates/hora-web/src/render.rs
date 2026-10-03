//! Server-rendered SVG: the owl, the daily bar, the latency sparkline and the
//! status/uptime badges.

use std::fmt::Write as _;

use axum::http::header;
use axum::response::IntoResponse;
use badgelib::{Badge, Color, Style};

use hora_core::db::{EventMarker, Point};
use hora_core::fmt::xml_escape;

// --- The owl ------------------------------------------------------------
// HoraFace: outlined (surface fill, ink line) so it follows the theme; its
// pose is the page's state, set by `data-state` and drawn by the stylesheet.
// It is never the only carrier of the state, so it is `aria-hidden`.

pub(crate) const OWL_PARTS: &str = concat!(
    r#"<path class="ln tl" d="M17.6 16.8C15.4 15.9 13.9 14 13.4 11.3 15.9 11.7 18.4 13 20.2 15.2Z"/>"#,
    r#"<path class="ln tr" d="M30.4 16.8C32.6 15.9 34.1 14 34.6 11.3 32.1 11.7 29.6 13 27.8 15.2Z"/>"#,
    r#"<ellipse class="ft" cx="20.6" cy="41.1" rx="1.9" ry="1.2"/><ellipse class="ft" cx="27.4" cy="41.1" rx="1.9" ry="1.2"/>"#,
    r#"<path class="ln" d="M24 14C30.9 14 35.6 19.2 35.6 26.6 35.6 34.6 30.6 40.4 24 40.4S12.4 34.6 12.4 26.6C12.4 19.2 17.1 14 24 14Z"/>"#,
    r#"<g class="tx"><circle cx="18.9" cy="24.6" r="4.9"/><circle cx="29.1" cy="24.6" r="4.9"/><path d="M14.4 30.8C15.8 33.7 18.1 36.3 21 37.8M33.6 30.8C32.2 33.7 29.9 36.3 27 37.8"/></g>"#,
    r#"<g class="ey"><g class="shut-l"><path d="M16.7 24.4a2.2 2.2 0 0 0 4.4 0"/></g><g class="shut-r"><path d="M26.9 24.4a2.2 2.2 0 0 0 4.4 0"/></g>"#,
    r#"<g class="open-l"><ellipse cx="18.9" cy="24.6" rx="2.2" ry="2.6"/><circle class="glint" cx="19.7" cy="23.6" r=".75"/></g>"#,
    r#"<g class="open-r"><ellipse cx="29.1" cy="24.6" rx="2.2" ry="2.6"/><circle class="glint" cx="29.9" cy="23.6" r=".75"/></g>"#,
    r#"<g class="dash"><path d="M16.9 24.8h4M27.1 24.8h4"/></g></g>"#,
    r#"<path class="bk" d="M22.9 28.3h2.2L24 30.1Z"/>"#,
    r#"<ellipse class="ck" cx="16.4" cy="30.6" rx="2" ry="1.2"/><ellipse class="ck" cx="31.6" cy="30.6" rx="2" ry="1.2"/>"#,
    r#"<g class="badge"><circle class="bd" cx="39.5" cy="38.5" r="7.2"/>"#,
    r##"<g class="b-degraded"><svg x="33.5" y="32.5" width="12" height="12"><use href="#g-degraded"/></svg></g>"##,
    r##"<g class="b-down"><svg x="33.5" y="32.5" width="12" height="12"><use href="#g-down"/></svg></g>"##,
    r##"<g class="b-maint"><svg x="33.5" y="32.5" width="12" height="12"><use href="#g-maint"/></svg></g>"##,
    r##"<g class="b-none"><svg x="33.5" y="32.5" width="12" height="12"><use href="#g-none"/></svg></g></g>"##,
);

/// The owl in the pose of `state` (`up`, `degraded`, `down`, `maint`,
/// `none`), `size` CSS pixels square. One per page, beside its sentence.
pub(crate) fn owl(state: &str, size: u32) -> String {
    format!(
        "<svg class=\"owl\" data-state=\"{state}\" viewBox=\"6 6 40 40\" width=\"{size}\" \
         height=\"{size}\" aria-hidden=\"true\" focusable=\"false\">{OWL_PARTS}</svg>"
    )
}

/// Several owls on one branch, one per place, each in its own pose: the mesh
/// drawn (the watchers page and the vantage panels only). `label` is the
/// sentence the picture says, for assistive tech.
pub(crate) fn owls(places: &[(&str, &str)], label: &str) -> String {
    const STEP: usize = 46;
    let width = STEP * places.len() + 8;
    let mut out = format!(
        "<svg class=\"owls\" viewBox=\"0 0 {width} 62\" role=\"img\" aria-label=\"{}\">\
         <path class=\"branch\" d=\"M0 45.5C{:.0} 43.5 {:.0} 47.5 {width} 44.5\"/>",
        xml_escape(label),
        coord(width) * 0.3,
        coord(width) * 0.7,
    );
    for (index, (state, _)) in places.iter().enumerate() {
        let x = 4 + index * STEP;
        let _ = write!(
            out,
            "<svg class=\"owl\" data-state=\"{state}\" x=\"{x}\" y=\"0\" width=\"46\" height=\"46\" \
             viewBox=\"6 6 40 40\">{OWL_PARTS}</svg>"
        );
    }
    for (index, (_, name)) in places.iter().enumerate() {
        let _ = write!(
            out,
            "<text class=\"olab\" x=\"{}\" y=\"58\" text-anchor=\"middle\">{}</text>",
            4 + index * STEP + 23,
            xml_escape(name),
        );
    }
    out.push_str("</svg>");
    out
}

// --- The daily bar ------------------------------------------------------
// ONE svg per monitor with a few run-length rects; the gaps between days are
// one mask shared by the whole page (`#gaps`, in the sprite). A day's height
// is part of its state, so the bar survives greyscale: up full height, slow
// shorter, down full and below the line, maintenance half, no data a stub.

/// A day's cell class in the bar: `u` up, `d` slow, `x` down, `m`
/// maintenance, `n` no data.
pub(crate) fn day_class(state: &str) -> u8 {
    match state {
        "up" => b'u',
        "degraded" => b'd',
        "down" => b'x',
        "maint" => b'm',
        _ => b'n',
    }
}

/// How bad a day cell is, to fold several monitors' days into a group's bar.
pub(crate) fn day_rank(class: u8) -> u8 {
    match class {
        b'x' => 4,
        b'd' => 3,
        b'm' => 2,
        b'u' => 1,
        _ => 0,
    }
}

/// Render the daily bar, oldest day first, from day cell classes.
pub(crate) fn day_bar(days: &[u8]) -> String {
    let mut rects = String::new();
    let mut start = 0;
    while let Some(&class) = days.get(start) {
        let run = days[start..]
            .iter()
            .take_while(|day| **day == class)
            .count();
        let (y, height) = match class {
            b'u' => (0, 22),
            b'd' => (7, 15),
            b'x' => (0, 29),
            b'm' => (11, 11),
            _ => (18, 4),
        };
        let _ = write!(
            rects,
            "<rect class=\"{}\" x=\"{start}\" y=\"{y}\" width=\"{run}\" height=\"{height}\"/>",
            char::from(class)
        );
        start += run;
    }
    format!(
        "<svg viewBox=\"0 0 {} 29\" preserveAspectRatio=\"none\" aria-hidden=\"true\" \
         focusable=\"false\"><g mask=\"url(#gaps)\">{rects}</g></svg>",
        days.len().max(1)
    )
}

/// A month's day strip for the report, first day first, from day cell
/// classes: run-length rects like [`day_bar`], on a 16-unit height (down
/// fills it, up stops short). The gaps between days are the stylesheet's
/// mask for the month's length (`d31`): a running month's strip keeps the
/// whole month's width, its days to come left empty.
pub(crate) fn day_strip(days: &[u8], month_days: usize) -> String {
    let mut rects = String::new();
    let mut start = 0;
    while let Some(&class) = days.get(start) {
        let run = days[start..]
            .iter()
            .take_while(|day| **day == class)
            .count();
        let (y, height) = match class {
            b'u' => (4, 12),
            b'd' => (8, 8),
            b'x' => (0, 16),
            b'm' => (10, 6),
            _ => (14, 2),
        };
        let _ = write!(
            rects,
            "<rect class=\"{}\" x=\"{start}\" y=\"{y}\" width=\"{run}\" height=\"{height}\"/>",
            char::from(class)
        );
        start += run;
    }
    let width = month_days.max(days.len()).max(1);
    format!(
        "<svg class=\"d{width}\" viewBox=\"0 0 {width} 16\" preserveAspectRatio=\"none\" \
         aria-hidden=\"true\" focusable=\"false\">{rects}</svg>"
    )
}

/// A thin horizontal meter (`value` of `max`), drawn as an svg so its width is
/// an attribute, not an inline style the CSP would refuse.
pub(crate) fn meter(value: i64, max: i64, class: &str) -> String {
    let filled = if max > 0 {
        (value.clamp(0, max) * 100) / max
    } else {
        0
    };
    format!(
        "<svg class=\"{class}\" viewBox=\"0 0 100 8\" preserveAspectRatio=\"none\" aria-hidden=\"true\" \
         focusable=\"false\"><rect class=\"track\" width=\"100\" height=\"8\"/>\
         <rect class=\"fill\" width=\"{filled}\" height=\"8\"/></svg>"
    )
}

// --- Server-rendered latency chart --------------------------------------
// Colours come from CSS (the `status` class on the <svg>), not inline here.

pub(crate) const CHART_W: f64 = 680.0;
pub(crate) const CHART_H: f64 = 120.0;
pub(crate) const CHART_PAD: f64 = 8.0;

/// Saturating conversion of any integer to an SVG coordinate (`f64`).
pub(crate) fn coord<T: TryInto<i32>>(value: T) -> f64 {
    f64::from(value.try_into().unwrap_or(i32::MAX))
}

/// Render the last-24h latency series as a self-contained inline SVG
/// sparkline. `events` overlays operator-recorded markers ("deploy api
/// v2.3") as vertical lines, positioned by time within the series' span -
/// empty for the public view, where deploy titles must not leak.
pub(crate) fn sparkline(points: &[Point], status: &str, events: &[EventMarker]) -> String {
    if points.is_empty() {
        // No viewBox: the stretched (`preserveAspectRatio="none"`) chart
        // coordinates would squash the text too. Without one, user units are
        // CSS pixels and the label is centred undistorted at any card width.
        return format!(
            "<svg class=\"spark {status}\">\
             <text x=\"50%\" y=\"50%\" class=\"spark-empty\" text-anchor=\"middle\" \
             dominant-baseline=\"middle\">no data yet</text></svg>"
        );
    }

    let count = points.len();
    let max = points
        .iter()
        .map(|p| p.latency_ms)
        .max()
        .unwrap_or(1)
        .max(1);
    let min = points.iter().map(|p| p.latency_ms).min().unwrap_or(0);
    let span = coord((max - min).max(1));
    let plot_h = CHART_H - 2.0 * CHART_PAD;
    let step = if count > 1 {
        (CHART_W - 2.0 * CHART_PAD) / (coord(count) - 1.0)
    } else {
        0.0
    };

    let mut line = String::new();
    for (index, point) in points.iter().enumerate() {
        let x = CHART_PAD + step * coord(index);
        let y = CHART_PAD + plot_h * (1.0 - coord(point.latency_ms - min) / span);
        let _ = write!(line, "{}{x:.1} {y:.1} ", if index == 0 { 'M' } else { 'L' });
    }

    let last_x = CHART_PAD + step * (coord(count) - 1.0);
    let baseline = CHART_H - CHART_PAD;
    let markers = event_markers(points, events);
    format!(
        "<svg viewBox=\"0 0 {CHART_W} {CHART_H}\" class=\"spark {status}\" preserveAspectRatio=\"none\">\
         <path class=\"spark-area\" d=\"{line}L{last_x:.1} {baseline:.1} L{CHART_PAD:.1} {baseline:.1} Z\"/>\
         <path class=\"spark-line\" d=\"{line}\"/>\
         {markers}</svg>"
    )
}

/// Vertical marker lines for the events falling inside the series' time span,
/// each carrying its title as a hover tooltip. The x position interpolates the
/// event's time between the first and last sample.
fn event_markers(points: &[Point], events: &[EventMarker]) -> String {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return String::new();
    };
    let span = coord((last.t - first.t).max(1));
    let plot_w = CHART_W - 2.0 * CHART_PAD;
    let mut out = String::new();
    for event in events
        .iter()
        .filter(|event| event.created_at >= first.t && event.created_at <= last.t)
    {
        let x = CHART_PAD + plot_w * (coord(event.created_at - first.t) / span);
        let _ = write!(
            out,
            "<line class=\"spark-event\" x1=\"{x:.1}\" x2=\"{x:.1}\" y1=\"{CHART_PAD:.1}\" \
             y2=\"{:.1}\"><title>{}</title></line>",
            CHART_H - CHART_PAD,
            xml_escape(&event.title),
        );
    }
    out
}

// --- SVG status / uptime badges -----------------------------------------

// The badges use the brand palette: up is petrol, not green. Text-grade
// shades, so the white message keeps its contrast on every colour.
const BADGE_UP: &str = "#0e5e68";
const BADGE_UP_SOFT: &str = "#3e7a84";
const BADGE_SLOW: &str = "#8f4b22";
const BADGE_DOWN: &str = "#a5372a";
/// A badge with no data yet (also the uptime badge's colour then).
pub(crate) const BADGE_NONE: &str = "#6b6760";

pub(crate) fn status_color(status: &str) -> &'static str {
    match status {
        "up" => BADGE_UP,
        "down" => BADGE_DOWN,
        "degraded" => BADGE_SLOW,
        _ => BADGE_NONE,
    }
}

pub(crate) fn uptime_color(permille: i64) -> &'static str {
    if permille >= 999 {
        BADGE_UP
    } else if permille >= 990 {
        BADGE_UP_SOFT
    } else if permille >= 900 {
        BADGE_SLOW
    } else {
        BADGE_DOWN
    }
}

pub(crate) fn svg_response(svg: String) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "public, max-age=60"),
        ],
        svg,
    )
}

/// Render a shields-style badge: a grey label and a coloured message.
pub(crate) fn badge(label: &str, message: &str, color: &str, style: Style) -> String {
    Badge::new()
        .label(label)
        .label_color(Color::Hex("1b222c".into()))
        .value(message)
        .value_color(Color::Hex(color.trim_start_matches('#').into()))
        .style(style)
        .to_svg()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sparkline_renders_svg_with_status_class() {
        let empty = sparkline(&[], "up", &[]);
        assert!(empty.contains("no data"));
        // The placeholder text must not be stretched with the chart.
        assert!(!empty.contains("preserveAspectRatio") && !empty.contains("viewBox"));
        let points = vec![
            Point {
                t: 1,
                latency_ms: 10,
            },
            Point {
                t: 2,
                latency_ms: 20,
            },
        ];
        let svg = sparkline(&points, "degraded", &[]);
        assert!(svg.contains("class=\"spark degraded\""));
        assert!(svg.contains("spark-line"));
    }

    #[test]
    fn sparkline_overlays_events_inside_the_span_only() {
        let points = vec![
            Point {
                t: 100,
                latency_ms: 10,
            },
            Point {
                t: 200,
                latency_ms: 20,
            },
        ];
        let events = vec![
            EventMarker {
                id: 1,
                title: "deploy <v2>".to_owned(),
                created_at: 150,
            },
            EventMarker {
                id: 2,
                title: "too old".to_owned(),
                created_at: 50,
            },
        ];
        let svg = sparkline(&points, "up", &events);
        // In-span marker drawn at the interpolated midpoint, title escaped.
        assert!(svg.contains("spark-event"), "{svg}");
        assert!(svg.contains("<title>deploy &lt;v2&gt;</title>"), "{svg}");
        let mid = CHART_PAD + (CHART_W - 2.0 * CHART_PAD) / 2.0;
        assert!(svg.contains(&format!("x1=\"{mid:.1}\"")), "{svg}");
        // Out-of-span events never render.
        assert!(!svg.contains("too old"), "{svg}");
    }

    #[test]
    fn badge_has_label_message_and_color() {
        let svg = badge("status", "up", status_color("up"), Style::Flat);
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains(">status<") && svg.contains(">up<"));
        assert!(svg.contains(&Color::Hex(status_color("up").into()).to_css()));
    }

    #[test]
    fn badge_supports_styles() {
        let flat = badge("status", "up", status_color("up"), Style::Flat);
        let flat_square = badge("status", "up", status_color("up"), Style::FlatSquare);
        let for_the_badge = badge("status", "up", status_color("up"), Style::ForTheBadge);

        assert!(flat.contains(r#"id="s""#));
        assert!(!flat_square.contains(r#"id="s""#));
        assert!(for_the_badge.contains(r#"height="28""#));
    }
    #[test]
    fn uptime_color_tiers() {
        assert_eq!(uptime_color(1000), BADGE_UP);
        assert_eq!(uptime_color(995), BADGE_UP_SOFT);
        assert_eq!(uptime_color(950), BADGE_SLOW);
        assert_eq!(uptime_color(800), BADGE_DOWN);
        // Up is petrol, never green.
        assert_eq!(status_color("up"), "#0e5e68");
    }

    #[test]
    fn day_bar_is_one_svg_of_run_length_rects() {
        let mut days = vec![b'u'; 90];
        days[40] = b'x';
        days[88] = b'd';
        days[89] = b'd';
        let svg = day_bar(&days);
        assert!(svg.starts_with("<svg viewBox=\"0 0 90 29\""), "{svg}");
        // up, down, up, slow: four runs, not ninety nodes.
        assert_eq!(svg.matches("<rect").count(), 4, "{svg}");
        assert!(
            svg.contains("class=\"x\" x=\"40\" y=\"0\" width=\"1\" height=\"29\""),
            "{svg}"
        );
        assert!(
            svg.contains("class=\"d\" x=\"88\" y=\"7\" width=\"2\""),
            "{svg}"
        );
        assert!(svg.contains("mask=\"url(#gaps)\""));
        // An empty bar still renders a valid (blank) svg.
        assert!(day_bar(&[]).contains("viewBox=\"0 0 1 29\""));
    }

    #[test]
    fn owl_is_decorative_and_posed_by_state() {
        let svg = owl("degraded", 96);
        assert!(svg.contains("data-state=\"degraded\"") && svg.contains("aria-hidden=\"true\""));
        let many = owls(
            &[("down", "Paris <1>"), ("up", "Frankfurt")],
            "Paris sees it down",
        );
        assert_eq!(many.matches("class=\"owl\"").count(), 2);
        assert!(many.contains("Paris &lt;1&gt;") && many.contains("role=\"img\""));
    }

    #[test]
    fn meter_width_is_an_attribute() {
        let svg = meter(31, 43, "meter");
        assert!(svg.contains("class=\"fill\" width=\"72\""), "{svg}");
        assert!(!svg.contains("style"));
        assert!(meter(5, 0, "meter").contains("width=\"0\""));
    }
}
