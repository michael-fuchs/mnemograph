//! Charts for the statistics dashboard, drawn as SVG on the server. The SVG stretches to the
//! chart's box and holds only the lines; labels are HTML around it, so text keeps its size at
//! any width.

use std::fmt::Write;

/// One line of a chart.
pub struct Series {
    pub label: String,
    /// A CSS color, e.g. `var(--accent)`.
    pub color: String,
    /// One value per point on the x axis.
    pub values: Vec<u32>,
}

/// A line chart of `series` over `x_labels` (one per point; only the first and last are shown).
/// `title` describes it for screen readers.
pub fn line_chart(title: &str, x_labels: &[String], series: &[Series]) -> String {
    let points = x_labels.len().max(1);
    let max = series
        .iter()
        .flat_map(|s| s.values.iter().copied())
        .max()
        .unwrap_or(0)
        .max(1);
    // x runs from 0 to the last point (at least 1, so a single week is a flat line); y from 0
    // at the top to 100 at the bottom.
    let width = (points - 1).max(1);
    let mut svg = format!(
        "<svg viewBox=\"0 0 {width} 100\" preserveAspectRatio=\"none\" role=\"img\" \
         aria-label=\"{}\">",
        escape(title)
    );
    for y in [0, 50, 100] {
        let _ = write!(
            svg,
            "<line class=\"grid\" x1=\"0\" x2=\"{width}\" y1=\"{y}\" y2=\"{y}\"/>"
        );
    }
    for line in series {
        let y = |v: u32| 100.0 - f64::from(v) * 100.0 / f64::from(max);
        let mut coordinates: Vec<String> = line
            .values
            .iter()
            .enumerate()
            .map(|(x, &v)| format!("{x},{:.2}", y(v)))
            .collect();
        if line.values.len() == 1 {
            coordinates.push(format!("1,{:.2}", y(line.values[0])));
        }
        let _ = write!(
            svg,
            "<polyline points=\"{}\" stroke=\"{}\"/>",
            coordinates.join(" "),
            escape(&line.color)
        );
    }
    svg.push_str("</svg>");

    let first = x_labels.first().map(String::as_str).unwrap_or("");
    let last = x_labels.last().map(String::as_str).unwrap_or("");
    let mut legend = String::new();
    for line in series {
        let latest = line.values.last().copied().unwrap_or(0);
        let _ = write!(
            legend,
            "<span><span class=\"swatch\" style=\"background: {}\"></span> {} ({latest})</span>",
            escape(&line.color),
            escape(&line.label)
        );
    }
    format!(
        "<figure class=\"chart\">\
           <div class=\"chart-plot\"><span class=\"chart-max\">{max}</span>{svg}</div>\
           <div class=\"chart-x\"><span>{}</span><span>{}</span></div>\
           <figcaption class=\"legend meta\">{legend}</figcaption>\
         </figure>",
        escape(first),
        if points > 1 {
            escape(last)
        } else {
            String::new()
        },
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(values: Vec<u32>) -> Series {
        Series {
            label: "Pages <b>".into(),
            color: "var(--accent)".into(),
            values,
        }
    }

    #[test]
    fn lines_are_scaled_to_the_largest_value() {
        let chart = line_chart(
            "Growth",
            &[
                "2026-10-05".into(),
                "2026-10-12".into(),
                "2026-10-19".into(),
            ],
            &[series(vec![0, 2, 4])],
        );
        assert!(chart.contains("viewBox=\"0 0 2 100\""), "{chart}");
        assert!(
            chart.contains("points=\"0,100.00 1,50.00 2,0.00\""),
            "{chart}"
        );
        assert!(chart.contains("<span class=\"chart-max\">4</span>"));
        assert!(chart.contains("<span>2026-10-05</span><span>2026-10-19</span>"));
        assert!(chart.contains("Pages &lt;b&gt; (4)"), "labels are escaped");
    }

    #[test]
    fn a_single_point_is_a_flat_line() {
        let chart = line_chart("Growth", &["2026-10-26".into()], &[series(vec![3])]);
        assert!(chart.contains("viewBox=\"0 0 1 100\""), "{chart}");
        assert!(chart.contains("points=\"0,0.00 1,0.00\""), "{chart}");
    }
}
