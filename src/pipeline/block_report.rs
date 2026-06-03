use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};

use crate::types::TokenCounts;

use super::*;

pub(super) fn build_block_json_report(
    loaded: LoadedUsage,
    tz: &TimeZoneMode,
    options: BlockReportBuildOptions,
) -> BlockJsonReport {
    let BlockReportBuildOptions {
        order,
        recent_only,
        active_only,
        window_secs,
        token_limit,
        token_limit_source,
        membership_estimate,
        official_codex,
        official_claude,
        official_antigravity,
        official_deepseek,
        official_openrouter,
        official_grok,
        official_kimi,
        official_anthropic_api,
        now,
    } = options;
    let mut grouped: HashMap<i64, GroupAggregate> = HashMap::new();
    for event in loaded.events {
        let unix = event.timestamp.timestamp();
        let block_start = unix - unix.rem_euclid(window_secs);
        grouped.entry(block_start).or_default().add_event(&event);
    }

    let mut grouped_blocks = grouped.into_iter().collect::<Vec<_>>();
    if recent_only {
        let recent_cutoff = now - chrono::TimeDelta::days(3);
        grouped_blocks.retain(|(start_unix, _)| {
            DateTime::from_timestamp(*start_unix, 0)
                .map(|dt| dt >= recent_cutoff)
                .unwrap_or(true)
        });
    }

    let mut blocks = grouped_blocks
        .into_iter()
        .map(|(start_unix, agg)| {
            let start = DateTime::from_timestamp(start_unix, 0).unwrap_or_else(Utc::now);
            let end = start + chrono::TimeDelta::seconds(window_secs);
            let is_active = now >= start && now < end;
            let percent_of_limit = token_limit.map(|limit| {
                if limit == 0 {
                    0.0
                } else {
                    (agg.totals.total_tokens() as f64 / limit as f64) * 100.0
                }
            });

            let row = BlockJsonRow {
                id: format!("{}", start.format("%Y%m%d%H")),
                start_time: format_display_datetime(start, tz),
                end_time: format_display_datetime(end, tz),
                is_active,
                totals: agg.totals.to_counts(),
                models: agg
                    .by_model
                    .into_iter()
                    .map(|(model, totals)| (model, totals.to_counts()))
                    .collect::<BTreeMap<_, _>>(),
                percent_of_limit,
            };

            (start_unix, row)
        })
        .collect::<Vec<_>>();

    if active_only {
        blocks.retain(|(_, row)| row.is_active);
    }

    blocks.sort_by_key(|(start_unix, _)| *start_unix);
    if order == SortOrder::Desc {
        blocks.reverse();
    }

    let blocks = blocks.into_iter().map(|(_, row)| row).collect::<Vec<_>>();
    let totals = blocks.iter().fold(TokenCounts::default(), |mut acc, row| {
        acc.add_assign(row.totals.clone());
        acc
    });

    BlockJsonReport {
        blocks,
        totals,
        stats: loaded.stats,
        token_limit,
        token_limit_source,
        membership_estimate,
        official_codex,
        official_claude,
        official_antigravity,
        official_deepseek,
        official_openrouter,
        official_grok,
        official_kimi,
        official_anthropic_api,
    }
}
