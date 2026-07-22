//! Terminal rendering for `tracker status`.
//!
//! Kept out of `tracker.rs` so the data model stays free of presentation: the
//! state exposes totals and rankings, this module decides how they look.

use std::collections::{BTreeMap, BTreeSet};

use crate::daemon::tracker::TrackerState;

/// Spacing between columns.
const GAP: &str = "  ";

enum Align {
    Left,
    Right,
}

/// Print a table whose columns are each sized to their own widest cell.
///
/// Widths are measured from the data rather than hard-coded. A hard-coded width
/// does not clip an oversized value — `{:>9}` on a ten-character string just
/// prints all ten — so one long cell would push every column after it out of
/// alignment for that row only. Measuring first means an unexpectedly large
/// number widens its column and the table still lines up.
///
/// Width is counted in `char`s rather than bytes. Everything rendered here is
/// ASCII — dates, digits, evdev key names — so one char is one column; if that
/// ever stops being true this needs real display widths (`unicode-width`),
/// since bytes would be wrong for any non-ASCII and chars wrong for CJK.
fn print_table(headers: &[&str], align: &[Align], rows: &[Vec<String>], footer: Option<&[String]>) {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows.iter().map(Vec::as_slice).chain(footer) {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }

    // Trailing spaces are trimmed so a left-aligned final column does not pad
    // out to the column width with nothing after it.
    let line = |cells: &[String]| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(i, cell)| match align[i] {
                Align::Left => format!("{cell:<width$}", width = widths[i]),
                Align::Right => format!("{cell:>width$}", width = widths[i]),
            })
            .collect::<Vec<String>>()
            .join(GAP)
            .trim_end()
            .to_string()
    };

    let header_cells: Vec<String> = headers.iter().map(|h| h.to_string()).collect();
    println!("{}", line(&header_cells));
    for row in rows {
        println!("{}", line(row));
    }

    if let Some(footer) = footer {
        let rule = widths.iter().sum::<usize>() + GAP.len() * widths.len().saturating_sub(1);
        println!("{}", "-".repeat(rule));
        println!("{}", line(footer));
    }
}

/// Summary table — one row per date, oldest first.
///
/// Anything above today is a day that was tracked but never pushed, which is
/// the whole reason state is bucketed per date.
pub fn brief(states: &BTreeMap<String, TrackerState>) {
    let headers = ["DATE", "KEYS", "CLICKS", "INCHES", "SCROLLS", "ACTIVE"];
    let align = [
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
        Align::Right,
    ];

    let mut keys = 0u64;
    let mut clicks = 0u64;
    let mut inches = 0f64;
    let mut scrolls = 0u64;
    let mut active = 0u64;

    let mut rows: Vec<Vec<String>> = Vec::with_capacity(states.len());
    for (date, state) in states {
        rows.push(vec![
            date.clone(),
            fmt_thousands(state.total_keys()),
            fmt_thousands(state.total_clicks()),
            fmt_inches(state.mouse_state.mouse_inches),
            fmt_thousands(state.mouse_state.mouse_scrolls as u64),
            fmt_duration(state.total_active_secs()),
        ]);

        keys += state.total_keys();
        clicks += state.total_clicks();
        inches += state.mouse_state.mouse_inches;
        scrolls += state.mouse_state.mouse_scrolls as u64;
        active += state.total_active_secs();
    }

    // A TOTAL row under a single date would just repeat that date's row.
    let footer = (states.len() > 1).then(|| {
        vec![
            "TOTAL".to_string(),
            fmt_thousands(keys),
            fmt_thousands(clicks),
            fmt_inches(inches),
            fmt_thousands(scrolls),
            fmt_duration(active),
        ]
    });

    print_table(&headers, &align, &rows, footer.as_deref());
}

/// Per-hour and per-key breakdown for every date.
pub fn detailed(states: &BTreeMap<String, TrackerState>) {
    for (date, state) in states {
        println!("### {date}");
        println!();

        // Hours come from the union of both maps: an hour can have active time
        // with no keystrokes (reading) or keystrokes with no completed active
        // tick, and dropping either would silently lose a row.
        let hours: BTreeSet<u8> = state
            .keyboard_state
            .keys()
            .chain(state.display_state.keys())
            .copied()
            .collect();

        let hour_rows: Vec<Vec<String>> = hours
            .iter()
            .map(|hour| {
                let keys: u64 = state
                    .keyboard_state
                    .get(hour)
                    .map(|keys| keys.values().map(|count| *count as u64).sum())
                    .unwrap_or(0);
                let active = state.display_state.get(hour).copied().unwrap_or(0);
                vec![
                    format!("{hour:02}:00"),
                    fmt_thousands(keys),
                    fmt_duration(active as u64),
                ]
            })
            .collect();
        print_table(
            &["HOUR", "KEYS", "ACTIVE"],
            &[Align::Left, Align::Right, Align::Right],
            &hour_rows,
            None,
        );

        println!();
        let key_rows: Vec<Vec<String>> = state
            .keys_ranked()
            .into_iter()
            .map(|(key, count)| vec![key.to_string(), fmt_thousands(count as u64)])
            .collect();
        print_table(
            &["KEY", "PRESSES"],
            &[Align::Left, Align::Right],
            &key_rows,
            None,
        );

        println!();
        let mouse = &state.mouse_state;
        let mouse_rows = vec![
            vec![
                "Left clicks".to_string(),
                fmt_thousands(mouse.left_click as u64),
            ],
            vec![
                "Right clicks".to_string(),
                fmt_thousands(mouse.right_click as u64),
            ],
            vec![
                "Middle clicks".to_string(),
                fmt_thousands(mouse.middle_click as u64),
            ],
            vec!["Inches moved".to_string(), fmt_inches(mouse.mouse_inches)],
            vec![
                "Scrolls".to_string(),
                fmt_thousands(mouse.mouse_scrolls as u64),
            ],
        ];
        print_table(
            &["MOUSE", "TOTAL"],
            &[Align::Left, Align::Right],
            &mouse_rows,
            None,
        );

        println!();
        println!("Active  {}", fmt_duration(state.total_active_secs()));
        println!();
    }
}

/// Group digits into thousands: `20433` -> `20,433`.
fn fmt_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        // A separator goes before every group of three counted from the right.
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Inches to one decimal, thousands-grouped: `2097.55` -> `2,097.6`.
///
/// `{:.1}` rounds first so the integer part is already final by the time it is
/// grouped — splitting the float by hand would have to redo that and get the
/// carry right.
fn fmt_inches(inches: f64) -> String {
    let rounded = format!("{:.1}", inches);
    let Some((whole, frac)) = rounded.split_once('.') else {
        return rounded;
    };
    match whole.parse::<u64>() {
        Ok(whole) => format!("{}.{}", fmt_thousands(whole), frac),
        // Not a plain number (NaN / inf): print whatever `{:.1}` produced.
        Err(_) => rounded.clone(),
    }
}

/// Seconds as a compact duration: `3h 5m`, `3h`, `42m`, `18s`.
///
/// Mirrors `formatDuration` in the dashboard's `src/lib/format.ts` so the CLI
/// and the web UI report the same number the same way.
fn fmt_duration(secs: u64) -> String {
    if secs == 0 {
        return "0m".to_string();
    }
    let hrs = secs / 3600;
    let mins = (secs % 3600) / 60;
    if hrs > 0 {
        if mins > 0 {
            return format!("{hrs}h {mins}m");
        }
        return format!("{hrs}h");
    }
    if mins > 0 {
        return format!("{mins}m");
    }
    format!("{secs}s")
}
