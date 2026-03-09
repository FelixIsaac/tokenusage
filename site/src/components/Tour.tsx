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
      className="inline-flex items-center justify-center w-6 h-6 rounded-md border border-[rgba(112,145,188,0.2)] bg-[rgba(10,22,38,0.5)] text-text-dim cursor-pointer transition-all hover:border-cyan/30 hover:text-text-soft"
    >
      {copied ? <Check size={11} /> : <Copy size={11} />}
    </span>
  );
}

const STEPS = [
  {
    key: "daily",
    index: "01",
    cmd: "tu",
    title: "Merged token report",
    desc: "Start with one command and get the classic multi-source token report. No activity columns are forced in by default, so the primary table stays compact and readable.",
    panel: {
      title: "Merged CLI report",
      link: "https://github.com/hanbu97/tokenusage#quick-start",
      linkLabel: "Quick start",
      img: "/assets/media/cli-demo-padded.png",
      alt: "CLI report demo",
    },
  },
  {
    key: "live",
    index: "02",
    cmd: "tu live codex",
    title: "Limits, pace, and forecast",
    desc: "See session limits, weekly limits, current pace, and projected exhaustion time — all updating in real time.",
    panel: {
      title: "Live monitor",
      link: "https://github.com/hanbu97/tokenusage#quick-start",
      linkLabel: "Live usage docs",
      img: "/assets/media/live-demo.png",
      alt: "Live monitor demo",
    },
  },
  {
    key: "activity",
    index: "03",
    cmd: "tu today",
    title: "Time views backed by local activity",
    desc: "Move from raw token counts to how work actually happened: coding time, tokens per hour, cost per hour, projects, models, and heartbeat-enhanced local activity.",
    panel: {
      title: "Activity and heartbeat",
      link: "https://github.com/hanbu97/tokenusage#how-does---with-activity-work",
      linkLabel: "Activity docs",
      content: {
        heading: "Native local activity layer",
        body: "`tu today`, `tu activity`, and `tu heartbeat` turn raw token events into a time-based view of how AI coding actually happens.",
        commands: ["tu today", "tu activity --days 14", "tu heartbeat watch .", "tu heartbeat stats"],
      },
    },
  },
  {
    key: "share",
    index: "04",
    cmd: "tu img week",
    title: "Share cards that market the product",
    desc: "Generate daily and weekly PNG share cards — ready to post on social media or drop into team updates.",
    panel: {
      title: "Share cards",
      link: "https://github.com/hanbu97/tokenusage#quick-start",
      linkLabel: "Image commands",
      img: "/assets/media/share-week-demo.png",
      alt: "Weekly share card demo",
    },
  },
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

  const activeStep = STEPS.find((s) => s.key === active)!;

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
          Four commands, one workflow.
        </h2>
        <p className="mt-4 text-text-soft leading-relaxed">
          Scroll through each stage to see how tokenusage fits together — from first report to share cards.
        </p>
      </motion.div>

      <div className="grid items-start gap-5.5 lg:grid-cols-[minmax(0,1.15fr)_minmax(280px,0.7fr)]">
        {/* Left: scrollable stage panels */}
        <div className="grid gap-6">
          {STEPS.map((step, i) => (
            <div
              key={step.key}
              ref={(el) => { stepRefs.current[i] = el; }}
              className="glass min-h-[380px] p-4.5 lg:min-h-[440px] transition-opacity duration-300 scroll-mt-24"
              style={{ opacity: active === step.key ? 1 : 0.4 }}
            >
              <div className="flex items-center justify-between gap-5 mb-4">
                <span className="font-[family-name:var(--font-display)] text-lg">{step.panel.title}</span>
                <a
                  href={step.panel.link}
                  target="_blank"
                  rel="noreferrer"
                  className="text-text-dim text-[0.92rem] hover:text-text-soft transition-colors"
                >
                  {step.panel.linkLabel}
                </a>
              </div>

              {step.panel.img && (
                <img
                  src={step.panel.img}
                  alt={step.panel.alt}
                  className="w-full h-[calc(100%-58px)] object-contain rounded-2xl border border-[rgba(122,146,186,0.18)] bg-[rgba(5,11,20,0.8)]"
                />
              )}

              {step.panel.content && (
                <div className="grid gap-4.5 h-[calc(100%-58px)] content-center">
                  <div className="p-6 rounded-[18px] bg-[rgba(9,19,34,0.9)] border border-line">
                    <h3 className="m-0 mb-3 font-[family-name:var(--font-display)] text-2xl">
                      {step.panel.content.heading}
                    </h3>
                    <p className="text-text-soft leading-relaxed">{step.panel.content.body}</p>
                  </div>
                  <div className="flex flex-wrap gap-2.5">
                    {step.panel.content.commands.map((c) => (
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
          ))}
        </div>

        {/* Right: sticky step nav */}
        <div className="sticky top-20 grid gap-4">
          {STEPS.map((step) => (
            <button
              key={step.key}
              type="button"
              onClick={() => handleClick(step.key)}
              className={`tour-step grid grid-cols-[48px_minmax(0,1fr)] gap-3 p-4 rounded-2xl border text-left cursor-pointer transition-all duration-300 ${
                active === step.key
                  ? "opacity-100 scale-100 border-cyan/34 bg-[linear-gradient(180deg,rgba(17,31,52,0.92),rgba(8,18,31,0.82))]"
                  : "opacity-48 scale-[0.985] border-[rgba(101,131,177,0.14)] bg-[linear-gradient(180deg,rgba(14,26,45,0.78),rgba(8,17,31,0.72))] hover:opacity-70 hover:border-cyan/20"
              }`}
            >
              <div className="inline-flex items-center justify-center w-[48px] h-[48px] rounded-[14px] border border-[rgba(115,147,196,0.2)] bg-[rgba(8,17,31,0.78)] text-cyan font-[family-name:var(--font-display)] text-sm">
                {step.index}
              </div>
              <div className="flex flex-col justify-center gap-1.5">
                <div className="inline-flex items-center gap-1.5 self-start">
                  <code className="px-2 py-1 rounded-lg bg-[rgba(8,17,30,0.82)] border border-[rgba(116,150,198,0.2)] text-cyan text-[0.75rem]">
                    {step.cmd}
                  </code>
                  <CmdCopy text={step.cmd} />
                </div>
                <h3 className="m-0 text-[0.95rem] leading-snug font-medium">
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
