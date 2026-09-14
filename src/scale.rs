use serde::{Deserialize, Serialize};

use crate::carbon::{format_commas_f64, format_commas_u64};
use crate::types::{TokenCounts, UsageEvent};

/// Human and psychological scale equivalences for token volume and generation.
///
/// Features a Dual-Lens perspective:
/// 1. **Active Creation** (Output Tokens) — tangible human production (books written, typing time).
/// 2. **Total Digestion / Cogitation** (Total Tokens including prompt caching) — total context
///    ingested and reasoned over across agentic cycles (Wikipedias, libraries, reading lifetimes, paper stacks).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InformationScale {
    /// Pure output / generated tokens
    pub output_tokens: u64,
    /// Estimated words generated (0.75 words per token)
    pub output_words: u64,
    /// Total tokens processed (Input + Output + Cache Reads + Cache Creation)
    pub total_tokens: u64,
    /// Estimated words digested (0.75 words per token)
    pub total_words: u64,

    // --- Active Creation metrics (Output tokens) ---
    /// Standard printed book pages written (~300 words/page)
    pub creation_pages: f64,
    /// Full-length novels written (~75,000 words/book)
    pub creation_books: f64,
    /// Complete Harry Potter 7-book series written (~1.08M words)
    pub creation_hp_series: f64,
    /// Human typing hours at 80 words per minute (4,800 words/hour)
    pub creation_typing_hours: f64,
    /// Human typing continuous years (24/7 @ 80 wpm)
    pub creation_typing_years_247: f64,

    // --- Total Digestion / Cogitation metrics (Total tokens) ---
    /// Complete Works of William Shakespeare (~884,647 words)
    pub digestion_shakespeare_works: f64,
    /// Complete 32-volume sets of Encyclopædia Britannica (~44M words)
    pub digestion_britannica_sets: f64,
    /// Multiples of the entire English Wikipedia (all ~6.8M articles, ~4.5B words)
    pub digestion_wikipedia_multiples: f64,
    /// 40,000-volume public city libraries cover-to-cover (~3.0B words)
    pub digestion_public_libraries: f64,
    /// Human reading hours at 250 words per minute (15,000 words/hour)
    pub digestion_reading_hours: f64,
    /// Human reading years at 8 hours/day (43.83M words/year)
    pub digestion_reading_years_8h: f64,
    /// Continuous 24/7 human reading lifetimes (75 years of nonstop 24/7 reading ≈ 9.86B words)
    pub digestion_reading_lifetimes_247: f64,
    /// Physical paper stack height in kilometers (double-sided 600 words/sheet @ 0.10 mm/sheet)
    pub digestion_paper_stack_km: f64,
}

impl InformationScale {
    pub fn calculate(output_tokens: u64, total_tokens: u64) -> Self {
        let output_words = (output_tokens as f64 * 0.75).round().max(0.0) as u64;
        let total_words = (total_tokens as f64 * 0.75).round().max(0.0) as u64;

        let out_w = output_words as f64;
        let tot_w = total_words as f64;

        // Creation metrics
        let creation_pages = out_w / 300.0;
        let creation_books = out_w / 75_000.0;
        let creation_hp_series = out_w / 1_084_170.0;
        let creation_typing_hours = out_w / 4_800.0; // 80 wpm * 60 min
        let creation_typing_years_247 = creation_typing_hours / (24.0 * 365.25);

        // Digestion metrics
        let digestion_shakespeare_works = tot_w / 884_647.0;
        let digestion_britannica_sets = tot_w / 44_000_000.0;
        let digestion_wikipedia_multiples = tot_w / 4_500_000_000.0;
        let digestion_public_libraries = tot_w / 3_000_000_000.0; // 40k books * 75k words
        let digestion_reading_hours = tot_w / 15_000.0; // 250 wpm * 60 min
        let digestion_reading_years_8h = tot_w / (15_000.0 * 8.0 * 365.25);
        let digestion_reading_lifetimes_247 = tot_w / (15_000.0 * 24.0 * 365.25 * 75.0);

        // Double-sided standard paper sheets (600 words/sheet, 0.10 mm thickness = 1e-7 km/sheet)
        let sheets = tot_w / 600.0;
        let digestion_paper_stack_km = (sheets * 0.0001) / 1000.0;

        Self {
            output_tokens,
            output_words,
            total_tokens,
            total_words,
            creation_pages,
            creation_books,
            creation_hp_series,
            creation_typing_hours,
            creation_typing_years_247,
            digestion_shakespeare_works,
            digestion_britannica_sets,
            digestion_wikipedia_multiples,
            digestion_public_libraries,
            digestion_reading_hours,
            digestion_reading_years_8h,
            digestion_reading_lifetimes_247,
            digestion_paper_stack_km,
        }
    }

    pub fn from_counts(counts: &TokenCounts) -> Self {
        let output = counts.output_tokens + counts.reasoning_output_tokens;
        Self::calculate(output, counts.total_tokens)
    }

    pub fn from_events(events: &[UsageEvent]) -> Self {
        let mut out = 0u64;
        let mut tot = 0u64;
        for e in events {
            out += e.usage.output_tokens + e.usage.reasoning_output_tokens;
            tot += e.usage.total_tokens();
        }
        Self::calculate(out, tot)
    }

    /// Dynamic 1–50× auto-stepping headline for Active Creation.
    pub fn creation_headline(&self) -> String {
        let words = self.output_words;
        if words < 1_000 {
            format!(
                "~{} words (~{:.0} paragraphs written)",
                format_commas_u64(words),
                (self.creation_pages * 3.0).max(1.0)
            )
        } else if words < 50_000 {
            format!(
                "~{} printed pages written (~{:.1}h typing @ 80 wpm)",
                format_commas_f64(self.creation_pages, 0),
                self.creation_typing_hours
            )
        } else if words < 1_000_000 {
            format!(
                "~{:.1} full-length novels written (~{:.1}h typing @ 80 wpm)",
                self.creation_books, self.creation_typing_hours
            )
        } else if self.creation_hp_series < 50.0 {
            format!(
                "~{} books written (~{:.1}× full Harry Potter series · {:.1}y non-stop typing)",
                format_commas_f64(self.creation_books, 0),
                self.creation_hp_series,
                self.creation_typing_years_247
            )
        } else {
            format!(
                "~{} full-length books written (~{:.1} years continuous typing @ 80 wpm)",
                format_commas_f64(self.creation_books, 0),
                self.creation_typing_years_247
            )
        }
    }

    /// Dynamic 1–50× auto-stepping headline for Total Digestion / Context.
    pub fn digestion_headline(&self) -> String {
        let words = self.total_words;
        if words < 50_000 {
            format!(
                "~{} pages of context (~{:.1}h reading time @ 250 wpm)",
                format_commas_f64(self.total_words as f64 / 300.0, 0),
                self.digestion_reading_hours
            )
        } else if words < 2_000_000 {
            format!(
                "~{:.1} full novels (~{:.1} Complete Shakespeare sets)",
                self.creation_books.max(self.total_words as f64 / 75_000.0),
                self.digestion_shakespeare_works
            )
        } else if words < 50_000_000 {
            format!(
                "~{:.1} Complete Works of Shakespeare (~{:.1} reading-months @ 8h/day)",
                self.digestion_shakespeare_works,
                self.digestion_reading_years_8h * 12.0
            )
        } else if words < 2_000_000_000 {
            format!(
                "~{:.1} complete 32-vol Encyclopædia Britannica sets (~{:.1} reading-years @ 8h/day)",
                self.digestion_britannica_sets, self.digestion_reading_years_8h
            )
        } else if self.digestion_wikipedia_multiples < 20.0 {
            format!(
                "~{:.1}× the entire English Wikipedia (~{} reading-years @ 8h/day)",
                self.digestion_wikipedia_multiples,
                format_commas_f64(self.digestion_reading_years_8h, 0)
            )
        } else {
            format!(
                "~{} public libraries cover-to-cover (~{:.1} human reading lifetimes 24/7)",
                format_commas_f64(self.digestion_public_libraries, 1),
                self.digestion_reading_lifetimes_247
            )
        }
    }

    /// Concise single-line summary for integration in reports or footer insights.
    pub fn compact_summary(&self) -> String {
        if self.total_tokens == 0 {
            return "-".to_string();
        }
        let dig = if self.digestion_wikipedia_multiples >= 0.5 {
            format!(
                "{:.1}× English Wikipedia",
                self.digestion_wikipedia_multiples
            )
        } else if self.digestion_britannica_sets >= 1.0 {
            format!("{:.0} Britannica sets", self.digestion_britannica_sets)
        } else if self.digestion_shakespeare_works >= 1.0 {
            format!(
                "{:.0} Complete Shakespeare",
                self.digestion_shakespeare_works
            )
        } else {
            format!(
                "{} pages",
                format_commas_f64(self.total_words as f64 / 300.0, 0)
            )
        };

        let cre = if self.creation_books >= 1.0 {
            format!(
                "{} books written",
                format_commas_f64(self.creation_books, 0)
            )
        } else {
            format!(
                "{} pages written",
                format_commas_f64(self.creation_pages, 0)
            )
        };

        format!("~{dig} digested · ~{cre}")
    }

    /// Structured multi-line card lines with rich comparative psychology details.
    pub fn detailed_card_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();

        lines.push(format!(
            "  ✍️  Active Generation ({} output tokens · ~{} words)",
            format_commas_u64(self.output_tokens),
            format_commas_u64(self.output_words)
        ));

        if self.creation_books >= 1.0 {
            if self.creation_hp_series >= 1.0 {
                lines.push(format!(
                    "     • Books Written:  ~{} full-length novels (e.g. {:.1}× full 7-book Harry Potter series)",
                    format_commas_f64(self.creation_books, 0),
                    self.creation_hp_series
                ));
            } else {
                lines.push(format!(
                    "     • Books Written:  ~{:.1} full-length novels (75k words each)",
                    self.creation_books
                ));
            }
        } else {
            lines.push(format!(
                "     • Pages Written:  ~{} printed book pages (~300 words/page)",
                format_commas_f64(self.creation_pages, 0)
            ));
        }

        if self.creation_typing_years_247 >= 0.1 {
            lines.push(format!(
                "     • Typing Effort:  ~{:.1} years of non-stop human typing (24/7 @ 80 wpm without pause)",
                self.creation_typing_years_247
            ));
        } else {
            lines.push(format!(
                "     • Typing Effort:  ~{} hours of continuous typing (@ 80 wpm)",
                format_commas_f64(self.creation_typing_hours, 1)
            ));
        }

        lines.push("".to_string());
        lines.push(format!(
            "  🧠  Total Context Digested ({} total tokens · ~{} words)",
            format_commas_u64(self.total_tokens),
            format_commas_u64(self.total_words)
        ));

        if self.digestion_wikipedia_multiples >= 0.2 {
            lines.push(format!(
                "     • World Knowledge:~{:.1}× the entirety of English Wikipedia (all 6.8M articles combined)",
                self.digestion_wikipedia_multiples
            ));
        } else if self.digestion_britannica_sets >= 0.5 {
            lines.push(format!(
                "     • Encyclopedias:  ~{:.1} complete 32-volume sets of Encyclopædia Britannica",
                self.digestion_britannica_sets
            ));
        } else if self.digestion_shakespeare_works >= 0.5 {
            lines.push(format!(
                "     • Classic Series: ~{:.1} Complete Works of William Shakespeare",
                self.digestion_shakespeare_works
            ));
        }

        if self.digestion_public_libraries >= 0.5 {
            lines.push(format!(
                "     • Library Scope:  ~{:.1} public city libraries (40,000 full volumes read cover-to-cover)",
                self.digestion_public_libraries
            ));
        }

        if self.digestion_reading_years_8h >= 1.0 {
            lines.push(format!(
                "     • Human Reading:  ~{} years of human reading time (8 hours/day @ 250 wpm)",
                format_commas_f64(self.digestion_reading_years_8h, 0)
            ));
        } else {
            lines.push(format!(
                "     • Human Reading:  ~{} hours of continuous reading (@ 250 wpm)",
                format_commas_f64(self.digestion_reading_hours, 0)
            ));
        }

        if self.digestion_paper_stack_km >= 0.05 {
            let note = if self.digestion_paper_stack_km >= 8.8 {
                " (higher than Mount Everest · jet cruising altitude)"
            } else if self.digestion_paper_stack_km >= 0.83 {
                " (higher than Burj Khalifa)"
            } else if self.digestion_paper_stack_km >= 0.38 {
                " (higher than Empire State Building)"
            } else {
                ""
            };
            lines.push(format!(
                "     • Physical Stack: ~{:.2} km tall printed double-sided on standard paper{}",
                self.digestion_paper_stack_km, note
            ));
        }

        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_small_scale() {
        let scale = InformationScale::calculate(500, 2_000);
        assert_eq!(scale.output_words, 375);
        assert_eq!(scale.total_words, 1_500);
        assert!(scale.creation_pages < 2.0);
        assert!(scale.digestion_reading_hours < 1.0);
    }

    #[test]
    fn test_medium_scale_novels() {
        let scale = InformationScale::calculate(100_000, 1_500_000);
        assert_eq!(scale.output_words, 75_000);
        assert_eq!(scale.creation_books, 1.0); // 1 full book
        assert!(scale.digestion_shakespeare_works > 1.0); // >1 Shakespeare
    }

    #[test]
    fn test_massive_scale_billions() {
        // User's 32 Billion tokens and 124.7M output tokens
        let scale = InformationScale::calculate(124_753_672, 32_005_227_742);
        assert_eq!(scale.output_words, 93_565_254);
        assert_eq!(scale.total_words, 24_003_920_807);
        // ~1247 books written
        assert!(scale.creation_books > 1200.0 && scale.creation_books < 1300.0);
        // ~5.33x English Wikipedia
        assert!((scale.digestion_wikipedia_multiples - 5.33).abs() < 1.0);
        // >500 reading years at 8h/day
        assert!(scale.digestion_reading_years_8h > 500.0);
        // ~4.0 km paper stack
        assert!(scale.digestion_paper_stack_km > 3.5);

        let lines = scale.detailed_card_lines();
        assert!(!lines.is_empty());
        let summary = scale.compact_summary();
        assert!(summary.contains("English Wikipedia") || summary.contains("books written"));
    }
}
