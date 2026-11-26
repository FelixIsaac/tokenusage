use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::carbon::format_commas_u64;
use crate::cli::WrappedTheme;
use crate::tui_theme::{format_compact_tokens, format_compact_usd};

use super::data::WrappedReport;

pub(crate) fn write_wrapped_html(
    path: &Path,
    report: &WrappedReport,
    theme: WrappedTheme,
) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }
    let html = render_wrapped_html(report, theme)?;
    fs::write(path, html).with_context(|| format!("Failed to write {}", path.display()))
}

pub(crate) fn render_wrapped_html(report: &WrappedReport, theme: WrappedTheme) -> Result<String> {
    let raw_json = serde_json::to_string(report)?;
    let safe_json = raw_json
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");

    let title = format!(
        "AI Year in Review {} · tokenusage wrapped",
        report.period_label
    );
    let total_tokens = format_compact_tokens(report.totals.total_tokens);
    let total_cost = format_compact_usd(report.totals.cost_usd);
    let saved_usd = format_compact_usd(report.cache.savings_usd);
    let cache_hit = format!("{:.1}%", report.cache.hit_pct);
    let reuse_ratio = format!("{:.1}×", report.cache.reuse_ratio);
    let output_words = format_commas_u64(report.scale.output_words);

    let html = HTML_TEMPLATE
        .replace("__TITLE__", &title)
        .replace("__PERIOD__", &report.period_label)
        .replace("__THEME__", theme.as_str())
        .replace("__TOTAL_TOKENS__", &total_tokens)
        .replace("__TOTAL_COST__", &total_cost)
        .replace("__SAVED_USD__", &saved_usd)
        .replace("__CACHE_HIT__", &cache_hit)
        .replace("__REUSE_RATIO__", &reuse_ratio)
        .replace("__OUTPUT_WORDS__", &output_words)
        .replace("__REPORT_JSON__", &safe_json);

    Ok(html)
}

const HTML_TEMPLATE: &str = r####"<!DOCTYPE html>
<html lang="en" data-theme="__THEME__">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>__TITLE__</title>
<link rel="preconnect" href="https://fonts.googleapis.com" />
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
<link href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@500;700;800&family=Outfit:wght@400;600;700;800;900&display=swap" rel="stylesheet" />
<script src="https://cdn.jsdelivr.net/npm/three@0.170.0/build/three.min.js"></script>
<script src="https://cdn.jsdelivr.net/npm/gsap@3.12.7/dist/gsap.min.js"></script>
<script src="https://cdn.jsdelivr.net/npm/gsap@3.12.7/dist/ScrollTrigger.min.js"></script>
<style>
  :root,
  [data-theme="cyber"] {
    --bg: #070a14;
    --shell: rgba(14, 20, 36, 0.78);
    --core: rgba(17, 25, 45, 0.88);
    --elevated: rgba(28, 39, 68, 0.9);
    --border: rgba(56, 189, 248, 0.22);
    --text: #f4f8ff;
    --muted: #9aaac4;
    --dim: #6c7c98;
    --primary: #00e5ff;
    --secondary: #a78bfa;
    --highlight: #fbbf24;
    --success: #10b981;
    --weave: none;
  }
  [data-theme="carbon"] {
    --bg: #090a0d;
    --shell: rgba(18, 20, 25, 0.82);
    --core: rgba(24, 27, 34, 0.92);
    --elevated: rgba(38, 42, 52, 0.92);
    --border: rgba(245, 158, 11, 0.28);
    --text: #f5f7fa;
    --muted: #b4b9c4;
    --dim: #7e8491;
    --primary: #f59e0b;
    --secondary: #f97316;
    --highlight: #fbbf24;
    --success: #34d399;
    --weave: repeating-linear-gradient(45deg, rgba(255,255,255,0.024) 0px, rgba(255,255,255,0.024) 2px, transparent 2px, transparent 6px),
             repeating-linear-gradient(-45deg, rgba(0,0,0,0.38) 0px, rgba(0,0,0,0.38) 2px, transparent 2px, transparent 6px);
  }
  [data-theme="matrix"] {
    --bg: #040b06;
    --shell: rgba(7, 19, 12, 0.80);
    --core: rgba(10, 26, 17, 0.90);
    --elevated: rgba(17, 42, 27, 0.92);
    --border: rgba(74, 222, 128, 0.25);
    --text: #ecfdf3;
    --muted: #86c29e;
    --dim: #548e6c;
    --primary: #4ade80;
    --secondary: #10b981;
    --highlight: #a3e635;
    --success: #34d399;
    --weave: none;
  }
  [data-theme="solar"] {
    --bg: #0c0816;
    --shell: rgba(22, 13, 34, 0.80);
    --core: rgba(30, 18, 46, 0.90);
    --elevated: rgba(48, 28, 68, 0.92);
    --border: rgba(251, 191, 36, 0.26);
    --text: #fff8f4;
    --muted: #ceb2c4;
    --dim: #947694;
    --primary: #fbbf24;
    --secondary: #fb7185;
    --highlight: #38bdf8;
    --success: #34d399;
    --weave: none;
  }

  * { box-sizing: border-box; }
  html, body {
    margin: 0; padding: 0; background-color: var(--bg); background-image: var(--weave);
    color: var(--text); font-family: "Outfit", ui-sans-serif, system-ui, sans-serif;
    line-height: 1.5; overflow-x: hidden; max-width: 100%;
  }
  .mono { font-family: "JetBrains Mono", ui-monospace, "SF Mono", Menlo, monospace; font-variant-numeric: tabular-nums; }
  #bg-stage {
    position: fixed; inset: 0; width: 100%; height: 100%; z-index: 0; pointer-events: none;
  }
  .film-grain {
    position: fixed; inset: 0; z-index: 40; pointer-events: none; opacity: 0.035;
    background-image: radial-gradient(rgba(255,255,255,0.6) 0.5px, transparent 0.5px);
    background-size: 3px 3px;
  }
  .page { position: relative; z-index: 2; padding: 24px 20px 120px; }
  .wrap { max-width: 1180px; margin: 0 auto; }

  /* Fluid Island Nav */
  .island-nav {
    position: sticky; top: 16px; z-index: 30; margin: 0 auto 36px; width: max-content; max-width: 100%;
    display: flex; flex-wrap: wrap; align-items: center; gap: 10px;
    padding: 8px 14px; border-radius: 999px;
    background: var(--shell); border: 1px solid var(--border);
    backdrop-filter: blur(18px); box-shadow: 0 12px 40px rgba(0,0,0,0.45);
  }
  .island-brand { font-weight: 800; font-size: 12px; letter-spacing: 0.14em; color: var(--primary); padding: 0 8px; }
  .theme-switch { display: flex; gap: 4px; background: rgba(0,0,0,0.35); padding: 3px; border-radius: 999px; }
  .theme-btn {
    border: 0; background: transparent; color: var(--muted); font-size: 11px; font-weight: 700;
    padding: 6px 11px; border-radius: 999px; cursor: pointer; letter-spacing: 0.04em;
    transition: all 0.3s cubic-bezier(0.32, 0.72, 0, 1);
  }
  .theme-btn.active { background: var(--primary); color: #060911; }
  .privacy-pill {
    font-size: 11px; padding: 5px 10px; border-radius: 999px; font-weight: 700;
    color: var(--success); border: 1px solid rgba(16,185,129,0.35); background: rgba(16,185,129,0.08);
  }

  /* Double-Bezel Cards */
  .bezel {
    background: var(--shell); border: 1px solid var(--border); border-radius: 28px; padding: 7px;
    transition: transform 0.55s cubic-bezier(0.32, 0.72, 0, 1), box-shadow 0.55s cubic-bezier(0.32, 0.72, 0, 1);
    transform-style: preserve-3d; position: relative;
  }
  .bezel:hover { box-shadow: 0 0 42px color-mix(in srgb, var(--primary) 22%, transparent); }
  .bezel-core {
    background: var(--core); border-radius: 21px; padding: 26px; height: 100%;
    box-shadow: inset 0 1px 0 rgba(255,255,255,0.08);
  }

  /* Hero */
  .hero-section {
    min-height: 78dvh; display: flex; flex-direction: column; justify-content: center; padding: 28px 0 44px;
  }
  .hero-kicker {
    display: inline-flex; align-items: center; gap: 10px; width: max-content; max-width: 100%;
    padding: 6px 14px; border-radius: 999px; font-size: 12px; font-weight: 800; letter-spacing: 0.14em;
    background: color-mix(in srgb, var(--highlight) 14%, transparent); color: var(--highlight);
    border: 1px solid color-mix(in srgb, var(--highlight) 35%, transparent); text-transform: uppercase;
  }
  .hero-h1 {
    font-size: clamp(3.2rem, 9vw, 6.8rem); font-weight: 900; line-height: 0.95; letter-spacing: -0.03em;
    margin: 22px 0 14px; max-width: 1120px;
    background: linear-gradient(120deg, var(--text) 10%, var(--primary) 55%, var(--secondary) 100%);
    -webkit-background-clip: text; color: transparent;
    text-shadow: 0 0 60px color-mix(in srgb, var(--primary) 25%, transparent);
  }
  .hero-sub { font-size: clamp(1rem, 2vw, 1.35rem); color: var(--muted); max-width: 780px; margin: 0 0 28px; }
  .hero-metrics { display: flex; flex-wrap: wrap; gap: 14px; }
  .hero-chip {
    padding: 10px 16px; border-radius: 14px; background: var(--shell); border: 1px solid var(--border);
    font-size: 13px; color: var(--muted);
  }
  .hero-chip strong { color: var(--text); }

  /* Infinite Marquee */
  .marquee-wrap {
    margin: 12px 0 36px; overflow: hidden; border-top: 1px solid var(--border); border-bottom: 1px solid var(--border);
    background: color-mix(in srgb, var(--shell) 70%, transparent); padding: 12px 0;
  }
  .marquee-track {
    display: flex; width: max-content; gap: 36px; white-space: nowrap;
    font-size: 13px; font-weight: 800; letter-spacing: 0.14em; color: var(--primary);
    animation: marquee-scroll 26s linear infinite;
  }
  @keyframes marquee-scroll {
    from { transform: translateX(0); }
    to { transform: translateX(-50%); }
  }

  /* Gapless Bento Grid */
  .bento {
    display: grid; grid-template-columns: repeat(12, 1fr); grid-auto-flow: dense; gap: 18px;
  }
  .span-3 { grid-column: span 3; }
  .span-4 { grid-column: span 4; }
  .span-6 { grid-column: span 6; }
  .span-8 { grid-column: span 8; }
  .span-12 { grid-column: span 12; }
  @media (max-width: 768px) {
    .span-3, .span-4, .span-6, .span-8, .span-12 { grid-column: span 12; }
  }

  .card-title { font-size: 12px; font-weight: 800; text-transform: uppercase; letter-spacing: 0.14em; color: var(--muted); margin: 0 0 14px; }
  .kpi-num { font-size: clamp(1.6rem, 3vw, 2.3rem); font-weight: 800; color: var(--text); line-height: 1.1; }
  .kpi-note { font-size: 12px; color: var(--dim); margin-top: 6px; }

  #city-3d {
    width: 100%; height: 340px; border-radius: 16px; overflow: hidden; position: relative;
    background: radial-gradient(circle at center, color-mix(in srgb, var(--primary) 10%, transparent), rgba(0,0,0,0.45));
    border: 1px solid var(--border); cursor: grab;
  }
  #city-3d:active { cursor: grabbing; }
  .city-hud {
    position: absolute; bottom: 12px; left: 14px; right: 14px; display: flex; justify-content: space-between;
    pointer-events: none; font-size: 11px; color: var(--muted);
  }

  .bar-row { margin-bottom: 13px; }
  .bar-meta { display: flex; justify-content: space-between; font-size: 13px; margin-bottom: 5px; }
  .bar-track { height: 9px; border-radius: 999px; background: rgba(0,0,0,0.42); overflow: hidden; }
  .bar-fill { height: 100%; border-radius: 999px; box-shadow: 0 0 12px currentColor; }

  .era-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(170px, 1fr)); gap: 12px; }
  .era-tile { background: rgba(0,0,0,0.32); border: 1px solid var(--border); border-radius: 14px; padding: 14px; }
  .badge-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(210px, 1fr)); gap: 12px; }
  .badge-tile {
    background: linear-gradient(145deg, rgba(255,255,255,0.04), rgba(0,0,0,0.35));
    border: 1px solid var(--border); border-radius: 14px; padding: 14px;
  }

  /* Floating Creator Dock */
  .dock {
    position: fixed; bottom: 18px; left: 50%; transform: translateX(-50%); z-index: 35;
    display: flex; flex-wrap: wrap; gap: 8px; justify-content: center; width: max-content; max-width: 96vw;
    padding: 10px 14px; border-radius: 999px; background: var(--shell); border: 1px solid var(--border);
    backdrop-filter: blur(18px); box-shadow: 0 18px 50px rgba(0,0,0,0.6);
  }
  .cta {
    display: inline-flex; align-items: center; gap: 8px; cursor: pointer; border-radius: 999px;
    border: 1px solid var(--border); background: var(--elevated); color: var(--text);
    padding: 8px 14px; font-weight: 700; font-size: 12px; font-family: inherit;
    transition: transform 0.25s cubic-bezier(0.32, 0.72, 0, 1), filter 0.25s;
  }
  .cta:hover { transform: translateY(-1px); filter: brightness(1.12); }
  .cta:active { transform: scale(0.97); }
  .cta.primary { background: var(--primary); color: #060911; border-color: var(--primary); }
  .cta-orb {
    width: 20px; height: 20px; border-radius: 999px; display: inline-flex; align-items: center; justify-content: center;
    background: rgba(0,0,0,0.18); font-size: 11px;
  }
</style>
</head>
<body>
<canvas id="bg-stage"></canvas>
<div class="film-grain"></div>

<div class="page">
  <div class="wrap">
    <nav class="island-nav">
      <span class="island-brand mono">TOKENUSAGE // __PERIOD__</span>
      <div class="theme-switch" id="theme-switch">
        <button class="theme-btn" data-set-theme="cyber">CYBER</button>
        <button class="theme-btn" data-set-theme="carbon">CARBON</button>
        <button class="theme-btn" data-set-theme="matrix">MATRIX</button>
        <button class="theme-btn" data-set-theme="solar">SOLAR</button>
      </div>
      <span class="privacy-pill mono" id="privacy-pill">PRIVACY SAFE</span>
    </nav>

    <section class="hero-section reveal">
      <div class="hero-kicker mono" id="hero-kicker"></div>
      <h1 class="hero-h1 mono" id="hero-counter">__TOTAL_TOKENS__ TOKENS</h1>
      <p class="hero-sub" id="hero-sub"></p>
      <div class="hero-metrics mono">
        <div class="hero-chip">API Value: <strong style="color:var(--success)">__TOTAL_COST__</strong></div>
        <div class="hero-chip">Cache Saved: <strong style="color:var(--primary)">~__SAVED_USD__</strong> (__CACHE_HIT__ · __REUSE_RATIO__)</div>
        <div class="hero-chip">Generated: <strong style="color:var(--highlight)">__OUTPUT_WORDS__ words</strong></div>
      </div>
    </section>

    <div class="marquee-wrap mono">
      <div class="marquee-track" id="marquee-track"></div>
    </div>

    <div class="bento">
      <div class="bezel span-3 tilt reveal"><div class="bezel-core" id="kpi-0"></div></div>
      <div class="bezel span-3 tilt reveal"><div class="bezel-core" id="kpi-1"></div></div>
      <div class="bezel span-3 tilt reveal"><div class="bezel-core" id="kpi-2"></div></div>
      <div class="bezel span-3 tilt reveal"><div class="bezel-core" id="kpi-3"></div></div>

      <div class="bezel span-8 reveal">
        <div class="bezel-core">
          <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:10px">
            <h2 class="card-title" style="margin:0">3D Token Cityscape (53-Week Extrusion)</h2>
            <button class="cta" id="spin-toggle" onclick="toggleCitySpin()" style="padding:4px 10px;font-size:11px">Pause Orbit</button>
          </div>
          <div id="city-3d">
            <div class="city-hud mono">
              <span>DRAG TO ORBIT · SCROLL TO ZOOM</span>
              <span id="city-peak-label"></span>
            </div>
          </div>
        </div>
      </div>

      <div class="bezel span-4 reveal">
        <div class="bezel-core">
          <h2 class="card-title">24h Circadian Equalizer</h2>
          <div id="hourly-chart"></div>
          <p id="chronotype-note" style="color:var(--muted);font-size:13px;margin:12px 0 0"></p>
        </div>
      </div>

      <div class="bezel span-6 reveal">
        <div class="bezel-core">
          <h2 class="card-title">Top Frontier Models</h2>
          <div id="models"></div>
        </div>
      </div>

      <div class="bezel span-6 reveal">
        <div class="bezel-core">
          <h2 class="card-title">Quarterly Eras &amp; Providers</h2>
          <div class="era-grid" id="eras" style="margin-bottom:18px"></div>
          <div id="providers"></div>
        </div>
      </div>

      <div class="bezel span-6 reveal">
        <div class="bezel-core">
          <h2 class="card-title">Pop-Culture Scale &amp; Planet Footprint</h2>
          <div id="scale-carbon"></div>
        </div>
      </div>

      <div class="bezel span-6 reveal">
        <div class="bezel-core">
          <h2 class="card-title">Unlocked Achievements</h2>
          <div class="badge-grid" id="badges"></div>
        </div>
      </div>
    </div>
  </div>
</div>

<div class="dock">
  <button class="cta primary" onclick="exportPosterPng()">Export Hype PNG <span class="cta-orb">↓</span></button>
  <button class="cta" id="rec-btn" onclick="recordMotionLoop()">Record 6s Loop (.webm) <span class="cta-orb">●</span></button>
  <button class="cta" onclick="copyText('share')">Copy X Post <span class="cta-orb">↗</span></button>
  <button class="cta" onclick="copyText('prompt')">AI Roast Prompt <span class="cta-orb">✦</span></button>
  <span id="dock-status" class="mono" style="align-self:center;font-size:11px;color:var(--success);padding:0 6px"></span>
</div>

<script>
const REPORT = __REPORT_JSON__;
const THEME_PALETTES = {
  cyber:  { bg: '#070a14', primary: '#00e5ff', secondary: '#a78bfa', highlight: '#fbbf24', success: '#10b981', weave: false },
  carbon: { bg: '#090a0d', primary: '#f59e0b', secondary: '#f97316', highlight: '#fbbf24', success: '#34d399', weave: true },
  matrix: { bg: '#040b06', primary: '#4ade80', secondary: '#10b981', highlight: '#a3e635', success: '#34d399', weave: false },
  solar:  { bg: '#0c0816', primary: '#fbbf24', secondary: '#fb7185', highlight: '#38bdf8', success: '#34d399', weave: false },
};
let activeTheme = document.documentElement.getAttribute('data-theme') || 'cyber';
let citySpinning = true;
let themeUpdateListeners = [];

function compactTok(n) {
  if (n >= 1e12) return (n/1e12).toFixed(1).replace(/\.0$/, '') + 'T';
  if (n >= 1e9) return (n/1e9).toFixed(1).replace(/\.0$/, '') + 'B';
  if (n >= 1e6) return (n/1e6).toFixed(1).replace(/\.0$/, '') + 'M';
  if (n >= 1e3) return (n/1e3).toFixed(1).replace(/\.0$/, '') + 'K';
  return String(n);
}
function usd(v) {
  return v >= 1000 ? '$' + Math.round(v).toLocaleString() : '$' + v.toFixed(2);
}
function esc(s) {
  return String(s).replace(/[&<>"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));
}
function provColor(name) {
  const s = name.toLowerCase();
  if (s.includes('claude')) return '#f97316';
  if (s.includes('codex') || s.includes('gpt')) return '#10b981';
  if (s.includes('gemini') || s.includes('antigravity')) return '#38bdf8';
  if (s.includes('opencode')) return '#a78bfa';
  if (s.includes('grok')) return '#fb7185';
  if (s.includes('pi')) return '#fbbf24';
  return THEME_PALETTES[activeTheme].primary;
}

// Theme switcher
function applyTheme(t) {
  activeTheme = THEME_PALETTES[t] ? t : 'cyber';
  document.documentElement.setAttribute('data-theme', activeTheme);
  document.querySelectorAll('.theme-btn').forEach(b => {
    b.classList.toggle('active', b.dataset.setTheme === activeTheme);
  });
  themeUpdateListeners.forEach(fn => fn(THEME_PALETTES[activeTheme]));
}
document.querySelectorAll('.theme-btn').forEach(btn => {
  btn.addEventListener('click', () => applyTheme(btn.dataset.setTheme));
});
applyTheme(activeTheme);

// Populate hero & marquee & KPIs
document.getElementById('privacy-pill').textContent = REPORT.include_projects ? 'PROJECTS OPT-IN' : 'PRIVACY SAFE';
document.getElementById('hero-kicker').textContent = REPORT.archetype.title + ' · ' + REPORT.archetype.tier;
document.getElementById('hero-sub').textContent =
  REPORT.pop_culture_scale + ' — ' + REPORT.archetype.tagline + ' ' + REPORT.archetype.reason;

const marqueeChunks = [
  `${compactTok(REPORT.totals.total_tokens)} TOKENS DIGESTED`,
  REPORT.archetype.title.toUpperCase(),
  REPORT.archetype.tier.toUpperCase(),
  REPORT.chronotype.label.toUpperCase(),
  `${REPORT.longest_streak_days}D STREAK`,
  REPORT.pop_culture_scale.toUpperCase(),
  `~${usd(REPORT.cache.savings_usd)} CACHE SAVED`,
];
const marqueeText = marqueeChunks.join('   ✦   ') + '   ✦   ';
document.getElementById('marquee-track').textContent = marqueeText + marqueeText;

const kpis = [
  ['Longest Streak', REPORT.longest_streak_days + ' DAYS', REPORT.active_days + ' active days logged', 'var(--highlight)'],
  ['Chronotype', REPORT.chronotype.label, 'Peak ' + String(REPORT.chronotype.peak_hour).padStart(2,'0') + ':00 · ' + REPORT.weekend_share_pct.toFixed(0) + '% weekend', 'var(--secondary)'],
  ['Cache Arbitrage', REPORT.cache.hit_pct.toFixed(1) + '% HIT', 'Saved ~' + usd(REPORT.cache.savings_usd) + ' (' + REPORT.cache.reuse_ratio.toFixed(1) + '× reuse)', 'var(--success)'],
  ['Peak Velocity Day', REPORT.peak_day ? compactTok(REPORT.peak_day.tokens) : '-', REPORT.peak_day ? REPORT.peak_day.date + ' · ' + usd(REPORT.peak_day.cost_usd) : '-', 'var(--primary)'],
];
kpis.forEach(([title, val, note, col], idx) => {
  document.getElementById('kpi-' + idx).innerHTML =
    `<h2 class="card-title">${esc(title)}</h2><div class="kpi-num mono" style="color:${col}">${esc(val)}</div><div class="kpi-note">${esc(note)}</div>`;
});

if (REPORT.peak_day) {
  document.getElementById('city-peak-label').textContent = `PEAK TOWER: ${compactTok(REPORT.peak_day.tokens)} (${REPORT.peak_day.date})`;
}

// 24h Equalizer SVG
(function renderHourly() {
  const max = Math.max(...REPORT.hourly_tokens, 1);
  const w = 480, h = 150, bw = 16, gap = 4;
  const bars = REPORT.hourly_tokens.map((v, i) => {
    const bh = Math.max(4, (v / max) * 120);
    const fill = i === REPORT.chronotype.peak_hour ? 'var(--highlight)' : 'var(--primary)';
    return `<rect x="${i*(bw+gap)}" y="${130-bh}" width="${bw}" height="${bh}" rx="4" fill="${fill}"><title>${String(i).padStart(2,'0')}:00 — ${compactTok(v)}</title></rect>`;
  }).join('');
  document.getElementById('hourly-chart').innerHTML = `<svg viewBox="0 0 ${w} ${h}" style="width:100%;height:auto">${bars}<text x="0" y="146" fill="var(--dim)" font-size="11">00:00</text><text x="220" y="146" fill="var(--dim)" font-size="11">12:00</text><text x="442" y="146" fill="var(--dim)" font-size="11">23:00</text></svg>`;
  document.getElementById('chronotype-note').textContent = REPORT.chronotype.detail;
})();

document.getElementById('models').innerHTML = REPORT.top_models.slice(0, 6).map(m =>
  `<div class="bar-row"><div class="bar-meta"><span>${esc(m.name)}</span><span class="mono" style="color:var(--muted)">${compactTok(m.tokens)} · ${m.share_pct.toFixed(1)}% · ${usd(m.cost_usd)}</span></div><div class="bar-track"><div class="bar-fill" style="width:${Math.min(100,m.share_pct)}%;background:${provColor(m.raw_name)};color:${provColor(m.raw_name)}"></div></div></div>`
).join('');

document.getElementById('eras').innerHTML = REPORT.quarterly_eras.map(q =>
  `<div class="era-tile"><div class="mono" style="color:var(--primary);font-size:11px;font-weight:800">${esc(q.quarter)}</div><div style="font-weight:800;margin:3px 0">${esc(q.title)}</div><div style="font-size:12px;color:var(--muted)">${esc(q.top_model)}</div><div class="mono" style="font-size:11px;color:var(--highlight);margin-top:6px">${compactTok(q.tokens)} · ${q.share_pct.toFixed(0)}%</div></div>`
).join('');

document.getElementById('providers').innerHTML = REPORT.top_providers.slice(0, 4).map(p =>
  `<div class="bar-row"><div class="bar-meta"><span>${esc(p.name)}</span><span class="mono" style="color:var(--muted)">${compactTok(p.tokens)} · ${p.share_pct.toFixed(1)}%</span></div><div class="bar-track"><div class="bar-fill" style="width:${Math.min(100,p.share_pct)}%;background:${provColor(p.name)};color:${provColor(p.name)}"></div></div></div>`
).join('');

document.getElementById('scale-carbon').innerHTML = `
  <p style="margin:0 0 12px;font-size:16px"><strong style="color:var(--primary)">${esc(REPORT.pop_culture_scale)}</strong></p>
  <p style="margin:0 0 12px;color:var(--muted)"><strong>Information Scale:</strong> ${esc(REPORT.scale_headline)} (${REPORT.scale.output_words.toLocaleString()} words generated).</p>
  <p style="margin:0 0 12px"><strong>Uncached Counterfactual:</strong> Would cost <span class="mono" style="color:var(--secondary)">${usd(REPORT.cache.uncached_cost_usd)}</span> vs <span class="mono" style="color:var(--success)">${usd(REPORT.totals.cost_usd)}</span> (coding-agent norm: ~91–96.6%).</p>
  <p style="margin:0;color:var(--dim)" class="mono">${esc(REPORT.carbon.summary)}</p>
`;

document.getElementById('badges').innerHTML = REPORT.badges.map(b =>
  `<div class="badge-tile"><div style="font-weight:800;color:var(--highlight);font-size:14px">${esc(b.title)}</div><div style="font-size:12px;color:var(--muted);margin-top:5px">${esc(b.detail)}</div></div>`
).join('');

// 3D Tilt foil feel on KPI cards
document.querySelectorAll('.tilt').forEach(card => {
  card.addEventListener('mousemove', e => {
    const r = card.getBoundingClientRect();
    const dx = (e.clientX - r.left) / r.width - 0.5;
    const dy = (e.clientY - r.top) / r.height - 0.5;
    card.style.transform = `perspective(900px) rotateX(${(-dy * 8).toFixed(2)}deg) rotateY(${(dx * 10).toFixed(2)}deg) translateY(-2px)`;
  });
  card.addEventListener('mouseleave', () => { card.style.transform = ''; });
});

// GSAP animations (safe when CDN unavailable)
if (window.gsap) {
  if (window.ScrollTrigger) gsap.registerPlugin(ScrollTrigger);
  const obj = { v: 0 };
  const targetTok = REPORT.totals.total_tokens;
  const heroEl = document.getElementById('hero-counter');
  gsap.to(obj, {
    v: targetTok, duration: 1.5, ease: 'power3.out',
    onUpdate: () => { heroEl.textContent = compactTok(Math.round(obj.v)) + ' TOKENS'; }
  });
  gsap.utils.toArray('.reveal').forEach((el, i) => {
    gsap.from(el, {
      y: 28, opacity: 0, duration: 0.75, ease: 'power3.out', delay: Math.min(i * 0.05, 0.3),
      scrollTrigger: window.ScrollTrigger ? { trigger: el, start: 'top 92%' } : undefined
    });
  });
}

// Background Three.js stage (with 2D neon fallback)
(function initBgStage() {
  const canvas = document.getElementById('bg-stage');
  let mouseX = 0, mouseY = 0;
  window.addEventListener('mousemove', e => {
    mouseX = (e.clientX / window.innerWidth - 0.5) * 2;
    mouseY = (e.clientY / window.innerHeight - 0.5) * 2;
  });

  if (window.THREE) {
    try {
      const renderer = new THREE.WebGLRenderer({ canvas, alpha: true, antialias: true });
      renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
      renderer.setSize(window.innerWidth, window.innerHeight);

      const scene = new THREE.Scene();
      const camera = new THREE.PerspectiveCamera(55, window.innerWidth / window.innerHeight, 0.1, 100);
      camera.position.z = 11;

      const coreMat = new THREE.MeshBasicMaterial({ color: THEME_PALETTES[activeTheme].primary, wireframe: true, transparent: true, opacity: 0.22 });
      const coreMesh = new THREE.Mesh(new THREE.TorusKnotGeometry(2.6, 0.75, 110, 16), coreMat);
      scene.add(coreMesh);

      const pCount = 1200;
      const pos = new Float32Array(pCount * 3);
      for (let i = 0; i < pCount; i++) {
        pos[i * 3] = (Math.random() - 0.5) * 30;
        pos[i * 3 + 1] = (Math.random() - 0.5) * 24;
        pos[i * 3 + 2] = (Math.random() - 0.5) * 20;
      }
      const pGeo = new THREE.BufferGeometry();
      pGeo.setAttribute('position', new THREE.BufferAttribute(pos, 3));
      const pMat = new THREE.PointsMaterial({ color: THEME_PALETTES[activeTheme].secondary, size: 0.08, transparent: true, opacity: 0.75 });
      const points = new THREE.Points(pGeo, pMat);
      scene.add(points);

      themeUpdateListeners.push(pal => {
        coreMat.color.set(pal.primary);
        pMat.color.set(pal.secondary);
      });

      window.addEventListener('resize', () => {
        camera.aspect = window.innerWidth / window.innerHeight;
        camera.updateProjectionMatrix();
        renderer.setSize(window.innerWidth, window.innerHeight);
      });

      function animateBg() {
        requestAnimationFrame(animateBg);
        const scrollK = window.scrollY * 0.0015;
        coreMesh.rotation.x += 0.003;
        coreMesh.rotation.y += 0.005;
        coreMesh.position.y = Math.sin(Date.now() * 0.001) * 0.35 - scrollK;
        points.rotation.y += 0.0008;
        camera.position.x += (mouseX * 1.1 - camera.position.x) * 0.04;
        camera.position.y += (-mouseY * 0.8 - camera.position.y) * 0.04;
        camera.lookAt(0, 0, 0);
        renderer.render(scene, camera);
      }
      animateBg();
      return;
    } catch (_) {}
  }

  // Offline / no-WebGL Canvas 2D neon plasma orb & particle fallback
  const ctx = canvas.getContext('2d');
  const resize = () => { canvas.width = window.innerWidth; canvas.height = window.innerHeight; };
  resize();
  window.addEventListener('resize', resize);
  const stars = Array.from({ length: 140 }, () => ({ x: Math.random(), y: Math.random(), s: Math.random() * 2 + 0.5, sp: Math.random() * 0.0006 + 0.0002 }));
  function draw2d() {
    requestAnimationFrame(draw2d);
    const pal = THEME_PALETTES[activeTheme];
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    stars.forEach(st => {
      st.y = (st.y + st.sp) % 1;
      ctx.fillStyle = pal.primary;
      ctx.globalAlpha = 0.25;
      ctx.fillRect(st.x * canvas.width, st.y * canvas.height, st.s, st.s);
    });
    ctx.globalAlpha = 1;
  }
  draw2d();
})();

// 3D Voxel Cityscape (heatmap extrusion) with SVG fallback
function toggleCitySpin() {
  citySpinning = !citySpinning;
  document.getElementById('spin-toggle').textContent = citySpinning ? 'Pause Orbit' : 'Resume Orbit';
}
(function initCity3D() {
  const container = document.getElementById('city-3d');
  if (window.THREE) {
    try {
      const w = container.clientWidth || 720, h = container.clientHeight || 340;
      const renderer = new THREE.WebGLRenderer({ alpha: true, antialias: true });
      renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
      renderer.setSize(w, h);
      container.prepend(renderer.domElement);

      const scene = new THREE.Scene();
      const camera = new THREE.PerspectiveCamera(42, w / h, 0.1, 200);
      const weeks = REPORT.heatmap.weeks || 53;
      const group = new THREE.Group();
      const gridFloor = new THREE.GridHelper(64, 32, 0x263147, 0x161d2e);
      gridFloor.position.y = -0.05;
      group.add(gridFloor);

      const dayLookup = new Map(REPORT.heatmap.days.map(d => [d.date, d]));
      const start = new Date(REPORT.heatmap.week_dates[0] + 'T00:00:00Z');
      const maxTok = Math.max(REPORT.heatmap.max_day_tokens || 1, 1);
      const barMatByLvl = [0,1,2,3,4].map(lvl => {
        const pal = THEME_PALETTES[activeTheme];
        const col = lvl === 4 ? pal.highlight : (lvl >= 2 ? pal.primary : pal.secondary);
        return new THREE.MeshStandardMaterial({
          color: col, emissive: col, emissiveIntensity: lvl * 0.18, roughness: 0.25, metalness: 0.7
        });
      });
      themeUpdateListeners.push(pal => {
        barMatByLvl.forEach((m, lvl) => {
          const col = lvl === 4 ? pal.highlight : (lvl >= 2 ? pal.primary : pal.secondary);
          m.color.set(col); m.emissive.set(col);
        });
      });

      const geo = new THREE.BoxGeometry(0.76, 1, 0.76);
      for (let col = 0; col < weeks; col++) {
        for (let row = 0; row < 7; row++) {
          const d = new Date(start.getTime() + (col * 7 + row) * 86400000);
          const iso = d.toISOString().slice(0, 10);
          const info = dayLookup.get(iso);
          const lvl = (REPORT.heatmap.grid[row] && REPORT.heatmap.grid[row][col]) || 0;
          const norm = info ? Math.pow(info.tokens / maxTok, 0.65) : 0;
          const bh = 0.15 + norm * 8.5;
          const mesh = new THREE.Mesh(geo, barMatByLvl[lvl]);
          mesh.scale.y = bh;
          mesh.position.set(col - weeks / 2, bh * 0.5, (row - 3) * 1.15);
          group.add(mesh);
        }
      }
      scene.add(group);
      scene.add(new THREE.AmbientLight(0xffffff, 0.75));
      const dl = new THREE.DirectionalLight(0xffffff, 1.4);
      dl.position.set(18, 28, 16);
      scene.add(dl);

      let yaw = -0.55, pitch = 0.52, zoom = 38, dragging = false, lastX = 0, lastY = 0;
      container.addEventListener('pointerdown', e => { dragging = true; lastX = e.clientX; lastY = e.clientY; });
      window.addEventListener('pointerup', () => { dragging = false; });
      window.addEventListener('pointermove', e => {
        if (!dragging) return;
        yaw -= (e.clientX - lastX) * 0.008;
        pitch = Math.max(0.18, Math.min(1.25, pitch + (e.clientY - lastY) * 0.006));
        lastX = e.clientX; lastY = e.clientY;
      });
      container.addEventListener('wheel', e => {
        e.preventDefault();
        zoom = Math.max(20, Math.min(68, zoom + e.deltaY * 0.03));
      }, { passive: false });

      window.addEventListener('resize', () => {
        const nw = container.clientWidth || 720, nh = container.clientHeight || 340;
        camera.aspect = nw / nh; camera.updateProjectionMatrix(); renderer.setSize(nw, nh);
      });

      function tickCity() {
        requestAnimationFrame(tickCity);
        if (citySpinning && !dragging) yaw += 0.003;
        camera.position.set(
          Math.cos(yaw) * Math.cos(pitch) * zoom,
          Math.sin(pitch) * zoom,
          Math.sin(yaw) * Math.cos(pitch) * zoom
        );
        camera.lookAt(0, 1.5, 0);
        renderer.render(scene, camera);
      }
      tickCity();
      return;
    } catch (_) {}
  }

  // Fallback glowing SVG matrix when WebGL/CDN unavailable
  const weeks = REPORT.heatmap.weeks || 53, cell = 12, gap = 3;
  let rects = '';
  for (let col = 0; col < weeks; col++) {
    for (let row = 0; row < 7; row++) {
      const lvl = (REPORT.heatmap.grid[row] && REPORT.heatmap.grid[row][col]) || 0;
      const op = [0.08, 0.28, 0.52, 0.78, 1.0][lvl];
      rects += `<rect x="${col*(cell+gap)+12}" y="${row*(cell+gap)+20}" width="${cell}" height="${cell}" rx="3" fill="var(--primary)" fill-opacity="${op}" />`;
    }
  }
  container.insertAdjacentHTML('afterbegin', `<svg viewBox="0 0 ${weeks*(cell+gap)+24} 160" style="width:100%;height:100%">${rects}</svg>`);
})();

function drawPosterToCanvas(canvas, tProgress = 1.0) {
  const ctx = canvas.getContext('2d');
  const pal = THEME_PALETTES[activeTheme];
  const W = 1200, H = 1500;
  ctx.fillStyle = pal.bg;
  ctx.fillRect(0, 0, W, H);
  if (pal.weave) {
    ctx.strokeStyle = 'rgba(255,255,255,0.03)';
    ctx.lineWidth = 2;
    for (let i = -H; i < W + H; i += 8) {
      ctx.beginPath(); ctx.moveTo(i, 0); ctx.lineTo(i + H, H); ctx.stroke();
    }
  }
  // Neon radial glows
  const g1 = ctx.createRadialGradient(220, 180, 10, 220, 180, 520);
  g1.addColorStop(0, pal.primary + '38'); g1.addColorStop(1, 'transparent');
  ctx.fillStyle = g1; ctx.fillRect(0, 0, W, H);

  ctx.strokeStyle = pal.primary + '55'; ctx.lineWidth = 3;
  ctx.strokeRect(32, 32, W - 64, H - 64);

  ctx.fillStyle = pal.primary; ctx.font = '800 24px "JetBrains Mono", monospace';
  ctx.fillText('TOKENUSAGE WRAPPED ' + REPORT.period_label + ' // ' + activeTheme.toUpperCase(), 76, 100);

  ctx.fillStyle = pal.highlight; ctx.font = '800 32px "Outfit", sans-serif';
  ctx.fillText(REPORT.archetype.title + ' · ' + REPORT.archetype.tier, 76, 165);

  ctx.save();
  ctx.shadowColor = pal.primary; ctx.shadowBlur = 32;
  ctx.fillStyle = '#ffffff'; ctx.font = '900 108px "JetBrains Mono", monospace';
  const curTok = Math.round(REPORT.totals.total_tokens * tProgress);
  ctx.fillText(compactTok(curTok) + ' TOKENS', 76, 308);
  ctx.restore();

  ctx.fillStyle = pal.success; ctx.font = '700 30px "JetBrains Mono", monospace';
  ctx.fillText(usd(REPORT.totals.cost_usd) + ' API value · ~' + usd(REPORT.cache.savings_usd) + ' saved (' + REPORT.cache.hit_pct.toFixed(1) + '% hit)', 76, 376);

  ctx.fillStyle = pal.primary; ctx.font = '700 24px "Outfit", sans-serif';
  ctx.fillText(REPORT.pop_culture_scale, 76, 434);

  // Pulse heatmap bars
  const cols = REPORT.heatmap.weeks || 53, cell = 16, gap = 4;
  const maxShowCols = Math.max(1, Math.floor(cols * Math.min(1, tProgress * 1.15)));
  for (let c = 0; c < maxShowCols; c++) {
    for (let r = 0; r < 7; r++) {
      const lvl = (REPORT.heatmap.grid[r] && REPORT.heatmap.grid[r][c]) || 0;
      ctx.fillStyle = lvl === 4 ? pal.highlight : pal.primary;
      ctx.globalAlpha = [0.1, 0.3, 0.55, 0.8, 1.0][lvl];
      ctx.fillRect(76 + c * (cell + gap), 510 + r * (cell + gap), cell, cell);
    }
  }
  ctx.globalAlpha = 1.0;

  ctx.fillStyle = '#e2e8f0'; ctx.font = '700 24px "Outfit", sans-serif';
  REPORT.top_models.slice(0, 5).forEach((m, i) => {
    const y = 740 + i * 68;
    ctx.fillStyle = '#e2e8f0';
    ctx.fillText(`#${i + 1} ${m.name} — ${compactTok(m.tokens)} (${m.share_pct.toFixed(0)}%)`, 76, y);
    ctx.fillStyle = 'rgba(255,255,255,0.08)'; ctx.fillRect(76, y + 14, 1048, 10);
    ctx.fillStyle = pal.primary; ctx.fillRect(76, y + 14, 1048 * (m.share_pct / 100) * tProgress, 10);
  });

  REPORT.badges.slice(0, 4).forEach((b, i) => {
    const bx = 76 + (i % 2) * 530, by = 1130 + Math.floor(i / 2) * 84;
    ctx.strokeStyle = pal.secondary; ctx.lineWidth = 2; ctx.strokeRect(bx, by, 510, 66);
    ctx.fillStyle = pal.highlight; ctx.font = '800 20px "Outfit", sans-serif';
    ctx.fillText(b.title.toUpperCase(), bx + 20, by + 30);
    ctx.fillStyle = '#94a3b8'; ctx.font = '15px "JetBrains Mono", monospace';
    ctx.fillText(b.detail, bx + 20, by + 52);
  });

  ctx.fillStyle = pal.highlight; ctx.font = '800 22px "JetBrains Mono", monospace';
  ctx.fillText('brew install FelixIsaac/tap/tokenusage && tu wrapped --theme ' + activeTheme, 76, 1418);
}

function exportPosterPng() {
  const c = document.createElement('canvas'); c.width = 1200; c.height = 1500;
  drawPosterToCanvas(c, 1.0);
  const a = document.createElement('a');
  a.href = c.toDataURL('image/png');
  a.download = `tokenusage-wrapped-${REPORT.period_label}-${activeTheme}.png`;
  a.click();
  document.getElementById('dock-status').textContent = `Downloaded PNG (${activeTheme})`;
}

function recordMotionLoop() {
  const btn = document.getElementById('rec-btn');
  const status = document.getElementById('dock-status');
  const canvas = document.createElement('canvas');
  canvas.width = 1200; canvas.height = 1500;
  if (!canvas.captureStream || !window.MediaRecorder) {
    status.textContent = 'MediaRecorder unavailable in this browser';
    return;
  }
  const stream = canvas.captureStream(60);
  const mime = MediaRecorder.isTypeSupported('video/webm;codecs=vp9') ? 'video/webm;codecs=vp9' : 'video/webm';
  const rec = new MediaRecorder(stream, { mimeType: mime, videoBitsPerSecond: 6000000 });
  const chunks = [];
  rec.ondataavailable = e => { if (e.data.size > 0) chunks.push(e.data); };
  rec.onstop = () => {
    const blob = new Blob(chunks, { type: 'video/webm' });
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = `tokenusage-wrapped-${REPORT.period_label}-${activeTheme}.webm`;
    a.click();
    btn.disabled = false;
    status.textContent = 'Downloaded 6s 60fps .webm loop!';
  };

  btn.disabled = true;
  rec.start();
  const t0 = performance.now();
  const dur = 6000;
  function step(now) {
    const rawT = Math.min(1, (now - t0) / 3600);
    const eased = 1 - Math.pow(1 - rawT, 3);
    drawPosterToCanvas(canvas, eased);
    status.textContent = `Recording ${((now - t0) / 1000).toFixed(1)}s / 6.0s`;
    if (now - t0 < dur) {
      requestAnimationFrame(step);
    } else {
      rec.stop();
    }
  }
  requestAnimationFrame(step);
}

function copyText(kind) {
  const txt = kind === 'share' ? REPORT.share_blurb : REPORT.agent_prompt;
  navigator.clipboard.writeText(txt).then(() => {
    document.getElementById('dock-status').textContent = kind === 'share' ? 'Copied X flex post!' : 'Copied AI prompt!';
  });
}
</script>
</body>
</html>
"####;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::CommonArgs;
    use crate::pipeline::TimeZoneMode;
    use crate::types::PricingTable;
    use crate::wrapped::data::{WrappedBuildOptions, build_wrapped_bundle};

    #[test]
    fn html_output_escapes_script_tags_and_includes_threejs_gsap_themes() {
        let tz = TimeZoneMode::Utc;
        let pricing = PricingTable::default_table();
        let common = CommonArgs::default();
        let bundle = build_wrapped_bundle(WrappedBuildOptions {
            events: &[],
            tz: &tz,
            timezone_label: "UTC</script>".to_string(),
            pricing: &pricing,
            target_year: Some(2026),
            all_time: false,
            include_projects: false,
            common: Some(&common),
        });
        let html = render_wrapped_html(&bundle.report, WrappedTheme::Carbon).unwrap();
        assert!(!html.contains("UTC</script>"));
        assert!(html.contains("\\u003c/script\\u003e"));
        assert!(html.contains("data-theme=\"carbon\""));
        assert!(html.contains("three@0.170.0"));
        assert!(html.contains("gsap@3.12.7"));
        assert!(html.contains("Record 6s Loop (.webm)"));
        assert!(html.contains("Export Hype PNG"));
    }
}
