//! Terminal output: the same data, either as a human table or as JSON.
//!
//! Every command funnels its output through [`Printer`] so `--json` is a
//! one-line change at the call site instead of a second code path.

use anyhow::Result;
use serde::Serialize;

/// How the CLI should render results.
pub struct Printer {
    json: bool,
}

impl Printer {
    pub fn new(json: bool) -> Self {
        Self { json }
    }

    pub fn is_json(&self) -> bool {
        self.json
    }

    /// Print one value as pretty JSON.
    pub fn print_json<T: Serialize>(&self, value: &T) -> Result<()> {
        println!("{}", serde_json::to_string_pretty(value)?);
        Ok(())
    }

    /// A single `key: value` line (or a one-key JSON object).
    pub fn kv(&self, key: &str, value: &str) {
        if self.json {
            println!("{}", serde_json::json!({ key: value }));
        } else {
            println!("{key}: {value}");
        }
    }

    /// Print an aligned table. Empty tables print nothing so callers do not
    /// emit a lone header row.
    pub fn table(&self, headers: &[&str], rows: &[Vec<String>]) {
        if self.json {
            // Rows are already stringified; emit them as an array of objects
            // keyed by the lowercased header, skipping unnamed columns.
            let objects: Vec<serde_json::Map<String, serde_json::Value>> = rows
                .iter()
                .map(|row| {
                    let mut map = serde_json::Map::new();
                    for (i, cell) in row.iter().enumerate() {
                        let key = headers
                            .get(i)
                            .copied()
                            .filter(|h| !h.trim().is_empty())
                            .unwrap_or_else(|| match i {
                                0 => "id",
                                _ => "col",
                            })
                            .to_ascii_lowercase();
                        map.insert(key, serde_json::Value::String(cell.clone()));
                    }
                    map
                })
                .collect();
            if let Ok(s) = serde_json::to_string_pretty(&objects) {
                println!("{s}");
            }
            return;
        }

        if rows.is_empty() {
            return;
        }

        let widths = column_widths(headers, rows);
        print_row(headers, &widths);
        for row in rows {
            print_row(row, &widths);
        }
    }
}

/// Widest cell per column, measured in display columns so CJK text lines up.
fn column_widths(headers: &[&str], rows: &[Vec<String>]) -> Vec<usize> {
    let mut widths: Vec<usize> = headers.iter().map(|h| display_width(h)).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            let w = display_width(cell);
            match widths.get_mut(i) {
                Some(cur) => *cur = (*cur).max(w),
                None => widths.push(w),
            }
        }
    }
    widths
}

fn print_row(cells: &[impl AsRef<str>], widths: &[usize]) {
    let mut line = String::new();
    for (i, cell) in cells.iter().enumerate() {
        let w = widths.get(i).copied().unwrap_or(0);
        let text = cell.as_ref();
        let pad = w.saturating_sub(display_width(text));
        if i > 0 {
            line.push_str("  ");
        }
        line.push_str(text);
        line.push_str(&" ".repeat(pad));
    }
    println!("{}", line.trim_end());
}

/// Characters-wide, not bytes-wide: East Asian wide glyphs take two cells.
fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| if is_wide(c) { 2 } else { 1 })
        .sum()
}

fn is_wide(c: char) -> bool {
    matches!(
        c,
        '\u{1100}'..='\u{115F}'
            | '\u{2E80}'..='\u{A4CF}'
            | '\u{AC00}'..='\u{D7A3}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FE30}'..='\u{FE6F}'
            | '\u{FF00}'..='\u{FF60}'
            | '\u{FFE0}'..='\u{FFE6}'
            | '\u{20000}'..='\u{3FFFD}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_width_counts_cjk_as_two() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("收件箱"), 6);
        assert_eq!(display_width("a箱"), 3);
    }

    #[test]
    fn widths_cover_every_column() {
        let rows = vec![
            vec!["a".to_string(), "bb".to_string(), "ccc".to_string()],
            vec!["aaaa".to_string()],
        ];
        let widths = column_widths(&["H1", "H2", "H3"], &rows);
        assert_eq!(widths, vec![4, 2, 3]);
    }
}
