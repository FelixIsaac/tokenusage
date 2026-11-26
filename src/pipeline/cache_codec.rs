use std::collections::HashMap;

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};

use crate::types::{CodexRawUsage, ServiceSpeed, UsageAccumulator};

use super::{CachedFileEntry, CachedFileStats, CachedUsageEvent, FileFingerprint};

const MAGIC: &[u8; 4] = b"TU05";

pub(super) fn encode_cached_file_entry(entry: &CachedFileEntry) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + entry.events.len() * 28);
    out.extend_from_slice(MAGIC);

    let has_codex = entry.codex_last_model.is_some()
        || entry.codex_last_totals.is_some()
        || entry.codex_service_tier != ServiceSpeed::Standard
        || entry.codex_fork_anchor_ms.is_some()
        || entry.codex_request_ledger_active;
    out.push(u8::from(has_codex));
    if has_codex {
        match &entry.codex_last_model {
            Some(m) => {
                out.push(1);
                write_str(&mut out, m);
            }
            None => out.push(0),
        }
        match entry.codex_last_totals {
            Some(t) => {
                out.push(1);
                write_u64(&mut out, t.input_tokens);
                write_u64(&mut out, t.cached_input_tokens);
                write_u64(&mut out, t.output_tokens);
                write_u64(&mut out, t.reasoning_output_tokens);
                write_u64(&mut out, t.total_tokens);
            }
            None => out.push(0),
        }
        out.push(speed_to_u8(entry.codex_service_tier));
        match entry.codex_fork_anchor_ms {
            Some(ms) => {
                out.push(1);
                write_i64(&mut out, ms);
            }
            None => out.push(0),
        }
        out.push(u8::from(entry.codex_request_ledger_active));
    }

    let mut models = Vec::<&str>::new();
    let mut model_map = HashMap::<&str, u32>::new();
    let mut sessions = Vec::<&str>::new();
    let mut session_map = HashMap::<&str, u32>::new();
    let mut projects = Vec::<&str>::new();
    let mut project_map = HashMap::<&str, u32>::new();

    for ev in &entry.events {
        intern(&mut models, &mut model_map, ev.model.as_str());
        if let Some(s) = ev.session.as_deref() {
            intern(&mut sessions, &mut session_map, s);
        }
        if let Some(p) = ev.project.as_deref() {
            intern(&mut projects, &mut project_map, p);
        }
    }

    write_str_table(&mut out, &models);
    write_str_table(&mut out, &sessions);
    write_str_table(&mut out, &projects);

    write_u64(&mut out, entry.events.len() as u64);
    let mut prev_ts_ms: i64 = 0;
    for (idx, ev) in entry.events.iter().enumerate() {
        let ts_ms = ev.timestamp.timestamp_millis();
        if idx == 0 {
            write_i64(&mut out, ts_ms);
        } else {
            write_i64(&mut out, ts_ms.wrapping_sub(prev_ts_ms));
        }
        prev_ts_ms = ts_ms;

        let model_idx = *model_map.get(ev.model.as_str()).unwrap_or(&0);
        let session_idx = ev
            .session
            .as_deref()
            .and_then(|s| session_map.get(s).map(|i| *i + 1))
            .unwrap_or(0);
        let project_idx = ev
            .project
            .as_deref()
            .and_then(|p| project_map.get(p).map(|i| *i + 1))
            .unwrap_or(0);
        write_u64(&mut out, u64::from(model_idx));
        write_u64(&mut out, u64::from(session_idx));
        write_u64(&mut out, u64::from(project_idx));

        let mut flags = speed_to_u8(ev.speed) & 0x03;
        if ev.cache_creation_1h_tokens > 0 {
            flags |= 0x04;
        }
        if ev.reported_cost_usd.is_some() {
            flags |= 0x08;
        }
        if ev.dedupe_hash.is_some() {
            flags |= 0x10;
        }
        if ev.model_calls > 1 {
            flags |= 0x20;
        }
        if ev.file_path.is_some() {
            flags |= 0x40;
        }
        out.push(flags);

        write_u64(&mut out, ev.usage.input_tokens);
        write_u64(&mut out, ev.usage.cache_creation_input_tokens);
        write_u64(&mut out, ev.usage.cache_read_input_tokens);
        write_u64(&mut out, ev.usage.output_tokens);
        write_u64(&mut out, ev.usage.reasoning_output_tokens);

        if ev.cache_creation_1h_tokens > 0 {
            write_u64(&mut out, ev.cache_creation_1h_tokens);
        }
        if let Some(cost) = ev.reported_cost_usd {
            out.extend_from_slice(&cost.to_le_bytes());
        }
        if let Some(hash) = ev.dedupe_hash {
            out.extend_from_slice(&hash.to_le_bytes());
        }
        if ev.model_calls > 1 {
            write_u64(&mut out, u64::from(ev.model_calls));
        }
        if let Some(fp) = ev.file_path.as_deref() {
            write_str(&mut out, fp);
        }
    }

    out
}

pub(super) fn decode_cached_file_entry(
    blob: &[u8],
    fingerprint: FileFingerprint,
    stats: CachedFileStats,
    parsed_offset: u64,
) -> Result<CachedFileEntry> {
    let mut r = Reader { data: blob, pos: 0 };
    if r.read_bytes(4)? != MAGIC {
        bail!("invalid TU05 cache magic");
    }

    let mut codex_last_model = None;
    let mut codex_last_totals = None;
    let mut codex_service_tier = ServiceSpeed::Standard;
    let mut codex_fork_anchor_ms = None;
    let mut codex_request_ledger_active = false;

    if r.read_u8()? != 0 {
        if r.read_u8()? != 0 {
            codex_last_model = Some(r.read_str()?);
        }
        if r.read_u8()? != 0 {
            codex_last_totals = Some(CodexRawUsage {
                input_tokens: r.read_u64()?,
                cached_input_tokens: r.read_u64()?,
                output_tokens: r.read_u64()?,
                reasoning_output_tokens: r.read_u64()?,
                total_tokens: r.read_u64()?,
            });
        }
        codex_service_tier = u8_to_speed(r.read_u8()?);
        if r.read_u8()? != 0 {
            codex_fork_anchor_ms = Some(r.read_i64()?);
        }
        codex_request_ledger_active = r.read_u8()? != 0;
    }

    let models = r.read_str_table()?;
    let sessions = r.read_str_table()?;
    let projects = r.read_str_table()?;

    let event_count = r.read_u64()? as usize;
    let mut events = Vec::with_capacity(event_count.min(65_536));
    let mut prev_ts_ms: i64 = 0;

    for idx in 0..event_count {
        let ts_ms = if idx == 0 {
            r.read_i64()?
        } else {
            prev_ts_ms.wrapping_add(r.read_i64()?)
        };
        prev_ts_ms = ts_ms;
        let timestamp =
            DateTime::from_timestamp_millis(ts_ms).unwrap_or(DateTime::<Utc>::UNIX_EPOCH);

        let model_idx = r.read_u64()? as usize;
        let session_idx = r.read_u64()? as usize;
        let project_idx = r.read_u64()? as usize;

        let model = models.get(model_idx).cloned().unwrap_or_default();
        let session = session_idx
            .checked_sub(1)
            .and_then(|i| sessions.get(i).cloned());
        let project = project_idx
            .checked_sub(1)
            .and_then(|i| projects.get(i).cloned());

        let flags = r.read_u8()?;
        let speed = u8_to_speed(flags & 0x03);
        let input_tokens = r.read_u64()?;
        let cache_creation_input_tokens = r.read_u64()?;
        let cache_read_input_tokens = r.read_u64()?;
        let output_tokens = r.read_u64()?;
        let reasoning_output_tokens = r.read_u64()?;

        let cache_creation_1h_tokens = if (flags & 0x04) != 0 {
            r.read_u64()?
        } else {
            0
        };
        let reported_cost_usd = if (flags & 0x08) != 0 {
            let b = r.read_bytes(8)?;
            Some(f64::from_le_bytes(b.try_into().unwrap()))
        } else {
            None
        };
        let dedupe_hash = if (flags & 0x10) != 0 {
            let b = r.read_bytes(8)?;
            Some(u64::from_le_bytes(b.try_into().unwrap()))
        } else {
            None
        };
        let model_calls = if (flags & 0x20) != 0 {
            (r.read_u64()? as u32).max(1)
        } else {
            1
        };
        let file_path = if (flags & 0x40) != 0 {
            Some(r.read_str()?)
        } else {
            None
        };

        events.push(CachedUsageEvent {
            timestamp,
            model,
            usage: UsageAccumulator {
                input_tokens,
                cache_creation_input_tokens,
                cache_read_input_tokens,
                output_tokens,
                reasoning_output_tokens,
                cost_usd: 0.0,
            },
            session,
            project,
            file_path,
            cache_creation_1h_tokens,
            speed,
            model_calls,
            reported_cost_usd,
            dedupe_hash,
        });
    }

    Ok(CachedFileEntry {
        fingerprint,
        stats,
        events,
        parsed_offset,
        codex_last_model,
        codex_last_totals,
        codex_service_tier,
        codex_fork_anchor_ms,
        codex_request_ledger_active,
    })
}

pub(super) fn entry_ts_bounds(entry: &CachedFileEntry) -> (i64, i64) {
    if entry.events.is_empty() {
        return (0, 0);
    }
    let mut min_ts = i64::MAX;
    let mut max_ts = i64::MIN;
    for ev in &entry.events {
        let ts = ev.timestamp.timestamp_millis();
        if ts < min_ts {
            min_ts = ts;
        }
        if ts > max_ts {
            max_ts = ts;
        }
    }
    (min_ts, max_ts)
}

fn intern<'a>(vec: &mut Vec<&'a str>, map: &mut HashMap<&'a str, u32>, val: &'a str) -> u32 {
    if let Some(&idx) = map.get(val) {
        return idx;
    }
    let idx = vec.len() as u32;
    vec.push(val);
    map.insert(val, idx);
    idx
}

fn speed_to_u8(speed: ServiceSpeed) -> u8 {
    match speed {
        ServiceSpeed::Standard => 0,
        ServiceSpeed::Fast => 1,
        ServiceSpeed::Ultrafast => 2,
    }
}

fn u8_to_speed(v: u8) -> ServiceSpeed {
    match v {
        1 => ServiceSpeed::Fast,
        2 => ServiceSpeed::Ultrafast,
        _ => ServiceSpeed::Standard,
    }
}

fn write_u64(out: &mut Vec<u8>, mut val: u64) {
    while val >= 0x80 {
        out.push(((val as u8) & 0x7f) | 0x80);
        val >>= 7;
    }
    out.push(val as u8);
}

fn write_i64(out: &mut Vec<u8>, val: i64) {
    let zigzag = ((val << 1) ^ (val >> 63)) as u64;
    write_u64(out, zigzag);
}

fn write_str(out: &mut Vec<u8>, s: &str) {
    write_u64(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

fn write_str_table(out: &mut Vec<u8>, table: &[&str]) {
    write_u64(out, table.len() as u64);
    for s in table {
        write_str(out, s);
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn read_u8(&mut self) -> Result<u8> {
        if self.pos >= self.data.len() {
            bail!("unexpected EOF in TU05 cache");
        }
        let b = self.data[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| anyhow::anyhow!("overflow"))?;
        if end > self.data.len() {
            bail!("unexpected EOF in TU05 cache");
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn read_u64(&mut self) -> Result<u64> {
        let mut res: u64 = 0;
        let mut shift = 0u32;
        loop {
            let b = self.read_u8()?;
            let val = u64::from(b & 0x7f);
            if shift >= 64 || (shift == 63 && val > 1) {
                bail!("varint overflow in TU05 cache");
            }
            res |= val << shift;
            if (b & 0x80) == 0 {
                return Ok(res);
            }
            shift += 7;
        }
    }

    fn read_i64(&mut self) -> Result<i64> {
        let zigzag = self.read_u64()?;
        Ok(((zigzag >> 1) as i64) ^ (-((zigzag & 1) as i64)))
    }

    fn read_str(&mut self) -> Result<String> {
        let len = self.read_u64()? as usize;
        let bytes = self.read_bytes(len)?;
        Ok(std::str::from_utf8(bytes)?.to_string())
    }

    fn read_str_table(&mut self) -> Result<Vec<String>> {
        let count = self.read_u64()? as usize;
        let mut out = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            out.push(self.read_str()?);
        }
        Ok(out)
    }
}
