import { useState, useEffect, useRef, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Copy, Check } from "lucide-react";

function CmdCopy({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);

  const copy = useCallback(async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      const area = document.createElement("textarea");
      area.value = text;
      area.style.position = "absolute";
      area.style.left = "-9999px";
      document.body.appendChild(area);
      area.select();
      document.execCommand("copy");
      document.body.removeChild(area);
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  }, [text]);

  return (
    <span
      role="button"
      onClick={copy}
      className="inline-flex items-center justify-center w-5 h-5 rounded-md border border-[rgba(112,145,188,0.2)] bg-[rgba(10,22,38,0.5)] text-text-dim cursor-pointer transition-all hover:border-cyan/30 hover:text-text-soft"
    >
      {copied ? <Check size={10} /> : <Copy size={10} />}
    </span>
  );
}

/* ---------- left-panel data (unchanged) ---------- */
const LEFT_PANELS: Record<string, {
  title: string; link: string; linkLabel: string;
  img?: string; alt?: string;
  images?: { src: string; alt: string }[];
  content?: { heading: string; body: string; commands: string[] };
}> = {
  daily: { title: "Merged CLI report", link: "https://github.com/hanbu97/tokenusage#quick-start", linkLabel: "Quick start", img: "/assets/media/cli-demo-padded.png", alt: "CLI report demo" },
  live:  { title: "Live monitor", link: "https://github.com/hanbu97/tokenusage#quick-start", linkLabel: "Live docs", img: "/assets/media/live-demo.png", alt: "Live monitor demo" },
  top:   { title: "Session top", link: "https://github.com/hanbu97/tokenusage#quick-start", linkLabel: "Top docs", content: { heading: "Per-session real-time view", body: "Streams token counts across all active sessions with configurable refresh and active window filtering.", commands: ["tu top", "tu top --active-hours 12", "tu top --active-hours 0"] } },
  today: { title: "Today view", link: "https://github.com/hanbu97/tokenusage#quick-start", linkLabel: "Activity docs", content: { heading: "Daily activity snapshot", body: "Turns raw token events into a time-based view of how AI coding happened today — projects, models, and cost per hour.", commands: ["tu today", "tu today --project myapp"] } },
  activity: { title: "Activity view", link: "https://github.com/hanbu97/tokenusage#how-does---with-activity-work", linkLabel: "Activity docs", content: { heading: "Coding activity over time", body: "Track how your AI usage evolves day-by-day with configurable date ranges and project filtering.", commands: ["tu activity", "tu activity --days 14", "tu activity --project tokenusage"] } },
  heartbeat: { title: "Heartbeat system", link: "https://github.com/hanbu97/tokenusage#quick-start", linkLabel: "Heartbeat docs", content: { heading: "Native local heartbeat", body: "Watch directories for file changes and build a local coding timeline — no cloud, no extensions needed.", commands: ["tu heartbeat watch .", "tu heartbeat stats", "tu heartbeat ping src/main.rs --write"] } },
  img:   { title: "Share cards", link: "https://github.com/hanbu97/tokenusage#quick-start", linkLabel: "Image docs", images: [{ src: "/assets/media/share-demo.png", alt: "Daily share card" }, { src: "/assets/media/share-week-demo.png", alt: "Weekly share card" }] },
  gui:   { title: "GUI dashboard", link: "https://github.com/hanbu97/tokenusage#quick-start", linkLabel: "GUI docs", img: "/assets/media/gui-demo.png", alt: "GUI dashboard demo" },
  periods: { title: "Period reports", link: "https://github.com/hanbu97/tokenusage#quick-start", linkLabel: "Period docs", content: { heading: "Flexible time aggregation", body: "Group usage by week or month with customizable week boundaries and full date range filtering.", commands: ["tu weekly", "tu weekly --start-of-week monday", "tu monthly"] } },
  statusline: { title: "Statusline", link: "https://github.com/hanbu97/tokenusage#quick-start", linkLabel: "Statusline docs", content: { heading: "Embed in your workflow", body: "Outputs a compact status string for tmux, Neovim, or any tool that reads shell output. Includes session cost, limits, and burn rate.", commands: ["tu statusline", "tu statusline --visual-burn-rate emoji"] } },
};

/* ---------- right-side steps (all features) ---------- */
const STEPS = [
  { key: "daily",      index: "01", cmd: "tu",            title: "Merged token report" },
  { key: "live",       index: "02", cmd: "tu live",       title: "Real-time TUI monitor" },
  { key: "top",        index: "03", cmd: "tu top",        title: "htop for tokens" },
  { key: "today",      index: "04", cmd: "tu today",      title: "Today's coding activity" },
  { key: "activity",   index: "05", cmd: "tu activity",   title: "Multi-day activity breakdown" },
  { key: "heartbeat",  index: "06", cmd: "tu heartbeat",  title: "Local heartbeat collector" },
  { key: "img",        index: "07", cmd: "tu img",        title: "Shareable image cards" },
  { key: "gui",        index: "08", cmd: "tu gui",        title: "Desktop GUI dashboard" },
  { key: "periods",    index: "09", cmd: "tu weekly",     title: "Weekly and monthly reports" },
  { key: "statusline", index: "10", cmd: "tu statusline", title: "Editor and tmux integration" },
];

export default function Tour() {
  const [active, setActive] = useState("daily");
  const sectionRef = useRef<HTMLElement>(null);
  const stepRefs = useRef<(HTMLDivElement | null)[]>([]);
  const manualRef = useRef(false);
  const manualTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const handleClick = useCallback((key: string) => {
    setActive(key);
    manualRef.current = true;
    if (manualTimer.current) clearTimeout(manualTimer.current);
    manualTimer.current = setTimeout(() => { manualRef.current = false; }, 1500);
    const idx = STEPS.findIndex((s) => s.key === key);
    const el = stepRefs.current[idx];
    if (el) el.scrollIntoView({ behavior: "smooth", block: "start" });
  }, []);

  // Scroll-driven activation: observe each left panel section
  useEffect(() => {
    const observers: IntersectionObserver[] = [];
    const keys = STEPS.map((s) => s.key);

    stepRefs.current.forEach((el, i) => {
      if (!el) return;
      const obs = new IntersectionObserver(
        ([entry]) => {
          if (entry.isIntersecting && !manualRef.current) {
            setActive(keys[i]);
          }
        },
        { rootMargin: "-35% 0px -35% 0px", threshold: 0.1 },
      );
      obs.observe(el);
      observers.push(obs);
    });

    return () => observers.forEach((o) => o.disconnect());
  }, []);

  return (
    <section id="tour" ref={sectionRef} className="mx-auto max-w-[min(1280px,calc(100vw-48px))] py-14">
      <motion.div
        className="max-w-[760px] mb-7"
        initial={{ opacity: 0, y: 20 }}
        whileInView={{ opacity: 1, y: 0 }}
        viewport={{ once: true, margin: "-10%" }}
        transition={{ duration: 0.5 }}
      >
        <span className="inline-block mb-3.5 px-3 py-2 rounded-full border border-cyan/28 bg-[rgba(9,23,40,0.7)] font-[family-name:var(--font-display)] text-[0.7rem] tracking-[0.12em] uppercase text-cyan">
          Tour
        </span>
        <h2 className="mt-0 font-[family-name:var(--font-display)] text-[clamp(1.8rem,3.5vw,2.8rem)] leading-tight">
          Every command, one workflow.
        </h2>
        <p className="mt-4 text-text-soft leading-relaxed">
          Scroll through each feature to see how tokenusage covers your entire AI coding workflow.
        </p>
      </motion.div>

      <div className="grid items-start gap-5.5 lg:grid-cols-[minmax(0,1.15fr)_minmax(280px,0.7fr)]">
        {/* Left: scrollable stage panels */}
        <div className="grid gap-6">
          {STEPS.map((step, i) => {
            const panel = LEFT_PANELS[step.key];
            return (
              <div
                key={step.key}
                ref={(el) => { stepRefs.current[i] = el; }}
                className="glass min-h-[380px] p-4.5 lg:min-h-[440px] transition-opacity duration-300 scroll-mt-24"
                style={{ opacity: active === step.key ? 1 : 0.4 }}
              >
                <div className="flex items-center justify-between gap-5 mb-4">
                  <span className="font-[family-name:var(--font-display)] text-lg">{panel.title}</span>
                  <a
                    href={panel.link}
                    target="_blank"
                    rel="noreferrer"
                    className="text-text-dim text-[0.92rem] hover:text-text-soft transition-colors"
                  >
                    {panel.linkLabel}
                  </a>
                </div>

                {panel.img && (
                  <img
                    src={panel.img}
                    alt={panel.alt}
                    className="w-full h-[calc(100%-58px)] object-contain rounded-2xl border border-[rgba(122,146,186,0.18)] bg-[rgba(5,11,20,0.8)]"
                  />
                )}

                {panel.images && (
                  <div className="grid grid-cols-2 gap-3 h-[calc(100%-58px)]">
                    {panel.images.map((img) => (
                      <img
                        key={img.src}
                        src={img.src}
                        alt={img.alt}
                        className="w-full h-full object-contain rounded-2xl border border-[rgba(122,146,186,0.18)] bg-[rgba(5,11,20,0.8)]"
                      />
                    ))}
                  </div>
                )}

                {panel.content && (
                  <div className="grid gap-4.5 h-[calc(100%-58px)] content-center">
                    <div className="p-6 rounded-[18px] bg-[rgba(9,19,34,0.9)] border border-line">
                      <h3 className="m-0 mb-3 font-[family-name:var(--font-display)] text-2xl">
                        {panel.content.heading}
                      </h3>
                      <p className="text-text-soft leading-relaxed">{panel.content.body}</p>
                    </div>
                    <div className="flex flex-wrap gap-2.5">
                      {panel.content.commands.map((c) => (
                        <code
                          key={c}
                          className="px-2.5 py-2.5 rounded-xl border border-[rgba(111,145,194,0.18)] bg-[rgba(8,18,31,0.82)] text-text-soft"
                        >
                          {c}
                        </code>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            );
          })}
        </div>

        {/* Right: sticky step nav */}
        <div className="sticky top-20 grid gap-2">
          {STEPS.map((step) => (
            <button
              key={step.key}
              type="button"
              onClick={() => handleClick(step.key)}
              className={`tour-step grid grid-cols-[36px_minmax(0,1fr)] gap-2.5 px-3 py-2.5 rounded-xl border text-left cursor-pointer transition-all duration-300 ${
                active === step.key
                  ? "opacity-100 scale-100 border-cyan/34 bg-[linear-gradient(180deg,rgba(17,31,52,0.92),rgba(8,18,31,0.82))]"
                  : "opacity-48 scale-[0.985] border-[rgba(101,131,177,0.14)] bg-[linear-gradient(180deg,rgba(14,26,45,0.78),rgba(8,17,31,0.72))] hover:opacity-70 hover:border-cyan/20"
              }`}
            >
              <div className="inline-flex items-center justify-center w-[36px] h-[36px] rounded-[10px] border border-[rgba(115,147,196,0.2)] bg-[rgba(8,17,31,0.78)] text-cyan font-[family-name:var(--font-display)] text-xs">
                {step.index}
              </div>
              <div className="flex flex-col justify-center gap-1">
                <div className="inline-flex items-center gap-1.5 self-start">
                  <code className="px-1.5 py-0.5 rounded-md bg-[rgba(8,17,30,0.82)] border border-[rgba(116,150,198,0.2)] text-cyan text-[0.7rem]">
                    {step.cmd}
                  </code>
                  <CmdCopy text={step.cmd} />
                </div>
                <h3 className="m-0 text-[0.84rem] leading-snug font-medium">
                  {step.title}
                </h3>
              </div>
            </button>
          ))}
        </div>
      </div>
    </section>
  );
}
