import { useState, useCallback, useRef, type ReactNode } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Github, ChevronDown, Copy, Check, Zap, Shield, Layers, Radio, Share2 } from "lucide-react";

function TiltCard({
  children,
  className = "",
  style,
  onClick,
}: {
  children: ReactNode;
  className?: string;
  style?: React.CSSProperties;
  onClick?: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [tilt, setTilt] = useState({ x: 0, y: 0 });
  const [hovering, setHovering] = useState(false);

  const handleMove = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const x = (e.clientX - rect.left) / rect.width - 0.5;
    const y = (e.clientY - rect.top) / rect.height - 0.5;
    setTilt({ x: y * -12, y: x * 12 });
  }, []);

  const handleLeave = useCallback(() => {
    setTilt({ x: 0, y: 0 });
    setHovering(false);
  }, []);

  return (
    <div
      ref={ref}
      className={`cursor-pointer ${className}`}
      style={{
        perspective: "800px",
        zIndex: hovering ? 100 : undefined,
        ...style,
      }}
      onMouseMove={handleMove}
      onMouseEnter={() => setHovering(true)}
      onMouseLeave={handleLeave}
      onClick={onClick}
    >
      <div
        className="glass relative overflow-hidden rounded-2xl h-full"
        style={{
          transform: `rotateX(${tilt.x}deg) rotateY(${tilt.y}deg) scale(${hovering ? 1.06 : 1})`,
          boxShadow: hovering
            ? "0 24px 60px rgba(0,0,0,0.45), 0 0 20px rgba(95,231,226,0.08)"
            : "0 8px 24px rgba(0,0,0,0.2)",
          borderColor: hovering ? "rgba(95,231,226,0.3)" : undefined,
          transition: hovering
            ? "transform 0.2s cubic-bezier(0.22,1,0.36,1), box-shadow 0.3s ease-out, border-color 0.2s ease-out"
            : "transform 0.45s cubic-bezier(0.22,1,0.36,1), box-shadow 0.4s ease-out, border-color 0.3s ease-out",
        }}
      >
        {children}
      </div>
    </div>
  );
}

const INSTALL_METHODS = [
  { key: "npm", label: "npm", cmds: ["npm install -g tokenusage", "npx tokenusage"] },
  { key: "cargo", label: "cargo", cmds: ["cargo install tokenusage --bin tu", "cargo binstall tokenusage --no-confirm"] },
  { key: "pip", label: "pip", cmds: ["pip install tokenusage"] },
];

/* Anthropic sparkle mark */
function AnthropicLogo({ size = 18 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor">
      <path d="M13.827 3.52h3.603L24 20.48h-3.603l-6.57-16.96zm-7.258 0H10.172L16.74 20.48H13.14L11.06 15.14H5.56l-2.057 5.34H0L6.569 3.52zM6.78 12.24h3.073L8.32 8.15 6.78 12.24z" />
    </svg>
  );
}

/* OpenAI hexagonal knot */
function OpenAILogo({ size = 18 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor">
      <path d="M22.282 9.821a5.985 5.985 0 0 0-.516-4.91 6.046 6.046 0 0 0-6.51-2.9A6.065 6.065 0 0 0 4.981 4.18a5.998 5.998 0 0 0-3.992 2.9 6.042 6.042 0 0 0 .743 7.097 5.98 5.98 0 0 0 .51 4.911 6.051 6.051 0 0 0 6.515 2.9A5.985 5.985 0 0 0 13.26 24a6.056 6.056 0 0 0 5.772-4.206 5.99 5.99 0 0 0 3.997-2.9 6.056 6.056 0 0 0-.747-7.073zM13.26 22.43a4.476 4.476 0 0 1-2.876-1.04l.141-.081 4.779-2.758a.795.795 0 0 0 .392-.681v-6.737l2.02 1.168a.071.071 0 0 1 .038.052v5.583a4.504 4.504 0 0 1-4.494 4.494zM3.6 18.304a4.47 4.47 0 0 1-.535-3.014l.142.085 4.783 2.759a.771.771 0 0 0 .78 0l5.843-3.369v2.332a.08.08 0 0 1-.033.062L9.74 19.95a4.5 4.5 0 0 1-6.14-1.646zM2.34 7.896a4.485 4.485 0 0 1 2.366-1.973V11.6a.766.766 0 0 0 .388.676l5.815 3.355-2.02 1.168a.076.076 0 0 1-.071 0l-4.83-2.786A4.504 4.504 0 0 1 2.34 7.872zm16.597 3.855l-5.833-3.387L15.119 7.2a.076.076 0 0 1 .071 0l4.83 2.791a4.494 4.494 0 0 1-.676 8.105v-5.678a.79.79 0 0 0-.407-.667zm2.01-3.023l-.141-.085-4.774-2.782a.776.776 0 0 0-.785 0L9.409 9.23V6.897a.066.066 0 0 1 .028-.061l4.83-2.787a4.5 4.5 0 0 1 6.68 4.66zm-12.64 4.135l-2.02-1.164a.08.08 0 0 1-.038-.057V6.075a4.5 4.5 0 0 1 7.375-3.453l-.142.08L8.704 5.46a.795.795 0 0 0-.393.681zm1.097-2.365l2.602-1.5 2.607 1.5v2.999l-2.597 1.5-2.607-1.5z" />
    </svg>
  );
}

/* Antigravity — upward rocket / arrow */
function AntigravityLogo({ size = 18 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M4.5 16.5c-1.5 1.26-2 5-2 5s3.74-.5 5-2c.71-.84.7-2.13-.09-2.91a2.18 2.18 0 0 0-2.91-.09z" />
      <path d="M12 15l-3-3a22 22 0 0 1 2-3.95A12.88 12.88 0 0 1 22 2c0 2.72-.78 7.5-6 11a22.35 22.35 0 0 1-4 2z" />
      <path d="M9 12H4s.55-3.03 2-4c1.62-1.08 5 0 5 0" />
      <path d="M12 15v5s3.03-.55 4-2c1.08-1.62 0-5 0-5" />
    </svg>
  );
}

interface Partner {
  name: string;
  accent: string;
  bg: string;
  border: string;
  logo: ReactNode;
}

const PARTNERS: Partner[] = [
  { name: "Claude Code", accent: "#D97757", bg: "rgba(217,119,87,0.06)", border: "rgba(217,119,87,0.22)", logo: <AnthropicLogo /> },
  { name: "OpenAI Codex", accent: "#10A37F", bg: "rgba(16,163,127,0.06)", border: "rgba(16,163,127,0.22)", logo: <OpenAILogo /> },
  { name: "Antigravity", accent: "#A78BFA", bg: "rgba(167,139,250,0.06)", border: "rgba(167,139,250,0.22)", logo: <AntigravityLogo /> },
];

interface FeatureTagData {
  label: string;
  icon: ReactNode;
  accent: string;
  border: string;
  bg: string;
  title: string;
  details: string[];
}

const TAG_STYLE = {
  accent: "#5FE7E2",
  border: "rgba(95,231,226,0.22)",
  bg: "rgba(95,231,226,0.05)",
};

const FEATURE_TAGS: FeatureTagData[] = [
  {
    label: "214x Faster",
    icon: <Zap size={12} />,
    ...TAG_STYLE,
    title: "Rust-native speed",
    details: [
      "214x faster than ccusage on real-world logs",
      "Zero-copy JSONL parsing pipeline",
      "Sub-second results on 100k+ log entries",
    ],
  },
  {
    label: "100% Local",
    icon: <Shield size={12} />,
    ...TAG_STYLE,
    title: "No network calls, ever",
    details: [
      "All parsing runs on your machine",
      "Logs never leave your disk",
      "Works offline — no API keys needed",
    ],
  },
  {
    label: "3 Sources",
    icon: <Layers size={12} />,
    ...TAG_STYLE,
    title: "Unified reporting",
    details: [
      "Claude Code + OpenAI Codex + Antigravity",
      "Single merged token report",
      "Cross-provider daily/weekly summaries",
    ],
  },
  {
    label: "Live Monitor",
    icon: <Radio size={12} />,
    ...TAG_STYLE,
    title: "Real-time tracking",
    details: [
      "tu live streams token usage as you code",
      "Rate limit warnings before you hit walls",
      "Heartbeat-backed activity time tracking",
    ],
  },
  {
    label: "Share Cards",
    icon: <Share2 size={12} />,
    ...TAG_STYLE,
    title: "Show off your usage",
    details: [
      "Generate beautiful PNG share cards",
      "tu img day / tu img week",
      "Perfect for social media and team updates",
    ],
  },
];

function FeatureTag({ tag }: { tag: FeatureTagData }) {
  const [hovered, setHovered] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  return (
    <div
      ref={ref}
      className="relative"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <span
        className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full border text-[0.75rem] font-medium cursor-pointer transition-all duration-200"
        style={{
          color: hovered ? tag.accent : "rgba(95,231,226,0.7)",
          borderColor: hovered ? tag.accent : tag.border,
          background: tag.bg,
          boxShadow: hovered ? `0 0 16px ${tag.bg}` : "none",
        }}
      >
        {tag.icon}
        {tag.label}
      </span>

      <AnimatePresence>
        {hovered && (
          <motion.div
            className="absolute left-0 top-full mt-2 z-50 w-[260px] rounded-xl border border-line-strong/30 bg-[rgba(8,17,31,0.97)] backdrop-blur-xl p-4 shadow-[0_20px_50px_rgba(0,0,0,0.5)]"
            initial={{ opacity: 0, y: -4, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -4, scale: 0.97 }}
            transition={{ duration: 0.15 }}
          >
            <div className="flex items-center gap-2 mb-2.5">
              <span style={{ color: tag.accent }}>{tag.icon}</span>
              <span
                className="font-[family-name:var(--font-display)] text-[0.82rem] font-medium tracking-wide"
                style={{ color: tag.accent }}
              >
                {tag.title}
              </span>
            </div>
            <ul className="m-0 p-0 list-none flex flex-col gap-1.5">
              {tag.details.map((d) => (
                <li key={d} className="flex items-start gap-2 text-[0.78rem] leading-relaxed text-text-soft">
                  <span
                    className="mt-[6px] w-1 h-1 rounded-full shrink-0"
                    style={{ background: tag.accent }}
                  />
                  {d}
                </li>
              ))}
            </ul>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

function ScrollIndicator() {
  return (
    <button
      onClick={() => document.getElementById("tour")?.scrollIntoView({ behavior: "smooth" })}
      className="flex flex-col items-center gap-1.5 group cursor-pointer bg-transparent border-none"
    >
      <span className="text-text-dim text-[0.68rem] font-[family-name:var(--font-display)] tracking-[0.2em] uppercase transition-colors group-hover:text-cyan/60">
        Explore
      </span>
      <div className="relative w-6 h-9 rounded-full border-2 border-text-dim/30 flex justify-center transition-border group-hover:border-cyan/40">
        <div
          className="w-1.5 h-1.5 rounded-full bg-cyan mt-2"
          style={{ animation: "scroll-dot 1.8s ease-in-out infinite" }}
        />
      </div>
      <ChevronDown size={16} className="text-text-dim/40 -mt-1 transition-colors group-hover:text-cyan/50" />
    </button>
  );
}

function PartnerStrip() {
  return (
    <div
      className="w-full"
    >
      <p className="text-center text-text-dim text-[0.72rem] font-[family-name:var(--font-display)] tracking-[0.18em] uppercase mb-3">
        Works with
      </p>
      <div className="flex items-center justify-center gap-5 flex-wrap">
        {PARTNERS.map((p) => (
          <div
            key={p.name}
            className="flex items-center gap-3.5 px-6 py-3 rounded-lg border transition-all hover:-translate-y-0.5"
            style={{ background: p.bg, borderColor: p.border }}
          >
            <span className="shrink-0" style={{ color: p.accent }}>
              {p.logo}
            </span>
            <div className="flex flex-col">
              <span
                className="font-[family-name:var(--font-display)] text-[0.84rem] font-medium tracking-wider whitespace-nowrap leading-tight"
                style={{ color: p.accent }}
              >
                {p.name}
              </span>
              <span className="text-text-dim text-[0.62rem] tracking-wider uppercase leading-tight mt-0.5">
                Supported
              </span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function CmdRow({ cmd }: { cmd: string }) {
  const [copied, setCopied] = useState(false);

  const copy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(cmd);
    } catch {
      const area = document.createElement("textarea");
      area.value = cmd;
      area.style.position = "absolute";
      area.style.left = "-9999px";
      document.body.appendChild(area);
      area.select();
      document.execCommand("copy");
      document.body.removeChild(area);
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  }, [cmd]);

  return (
    <div
      className="flex items-center gap-3 px-3.5 py-2.5 rounded-2xl border border-[rgba(100,131,176,0.18)] bg-[rgba(6,15,28,0.92)] transition-colors hover:border-cyan/20"
    >
      <code className="flex-1 text-[#dffef1] text-[0.88rem] leading-relaxed">{cmd}</code>
      <button
        onClick={copy}
        className="shrink-0 flex items-center gap-1 px-2 py-1 rounded-lg border border-[rgba(112,145,188,0.2)] bg-[rgba(10,22,38,0.5)] text-text-dim text-[0.68rem] cursor-pointer transition-all hover:border-cyan/30 hover:text-text-soft"
      >
        {copied ? <Check size={12} /> : <Copy size={12} />}
      </button>
    </div>
  );
}

export default function Hero() {
  const [activeTab, setActiveTab] = useState("npm");

  const activeCmds = INSTALL_METHODS.find((m) => m.key === activeTab)!.cmds;

  return (
    <section id="top" className="mx-auto max-w-[min(1280px,calc(100vw-48px))] min-h-[calc(100svh-3rem)] flex flex-col pb-3">
      <div className="flex-1 flex items-center">
      <div className="w-full grid items-center gap-9 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.15fr)]">
        {/* Copy side */}
        <div
          className="min-w-0 flex flex-col gap-[clamp(24px,4.5svh,52px)]"
        >
          <h1
            className="font-[family-name:var(--font-display)] text-[clamp(3rem,5.5vw,4.5rem)] leading-[0.9] tracking-[0.06em] text-text-primary font-bold"
            style={{ textShadow: "0 0 40px rgba(95,231,226,0.08)" }}
          >
            TOKENUSAGE
          </h1>

          <p className="max-w-[20ch] font-[family-name:var(--font-display)] text-[clamp(1.15rem,1.6vw,1.5rem)] leading-snug text-text-soft/90 font-normal tracking-wide">
            Tokens, limits, and activity in one{" "}
            <span className="text-cyan font-medium">fast local workflow.</span>
          </p>

          <div className="flex flex-wrap gap-2">
            {FEATURE_TAGS.map((tag) => (
              <FeatureTag key={tag.label} tag={tag} />
            ))}
          </div>

          <div className="flex flex-wrap gap-3 -mt-2">
            <a
              href="https://github.com/hanbu97/tokenusage"
              target="_blank"
              rel="noreferrer"
              className="inline-flex items-center gap-2.5 rounded-full border border-white/12 bg-[linear-gradient(180deg,rgba(30,34,42,0.96),rgba(9,11,15,0.96))] px-5 py-2.5 font-[family-name:var(--font-display)] text-[0.78rem] tracking-wider text-[#f5f8fc] shadow-[0_14px_34px_rgba(0,0,0,0.36)] transition-all hover:-translate-y-0.5 hover:border-cyan/30"
            >
              <Github size={15} />
              Star on GitHub
            </a>
          </div>

          {/* Install card — fixed height container to prevent layout shift */}
          <div style={{ height: "220px" }}>
          <div className="glass max-w-[40rem] p-3.5 pb-4" style={{
            background: "linear-gradient(180deg, rgba(12,24,42,0.96), rgba(7,16,30,0.94)), linear-gradient(135deg, rgba(95,231,226,0.08), transparent 50%)"
          }}>
            <div className="mb-2.5">
              <span className="block mb-0.5 text-cyan font-[family-name:var(--font-display)] text-[0.68rem] tracking-[0.14em] uppercase">
                Install
              </span>
              <span className="font-[family-name:var(--font-display)] text-[0.84rem] tracking-wide text-text-primary">
                One line, then run <code className="text-cyan">tu</code>
              </span>
            </div>

            <div className="flex flex-wrap gap-2 mb-2.5">
              {INSTALL_METHODS.map((m) => (
                <button
                  key={m.key}
                  onClick={() => setActiveTab(m.key)}
                  className={`min-h-[31px] px-2.5 rounded-full border font-[family-name:var(--font-display)] text-[0.72rem] tracking-wider cursor-pointer transition-all ${
                    activeTab === m.key
                      ? "border-cyan/30 bg-[rgba(15,29,48,0.9)] text-cyan"
                      : "border-[rgba(100,131,176,0.18)] bg-[rgba(8,17,31,0.72)] text-text-dim hover:border-cyan/30 hover:text-cyan"
                  }`}
                >
                  {m.label}
                </button>
              ))}
            </div>

            <div className="flex flex-col gap-2">
              {activeCmds.map((cmd) => (
                <CmdRow key={cmd} cmd={cmd} />
              ))}
            </div>
          </div>
          </div>
        </div>

        {/* Visual side — scattered overlapping cards */}
        <div
          className="hidden lg:block relative min-w-0"
          style={{ height: "clamp(400px, 50svh, 520px)" }}
        >
          {/* tu gui — main backdrop, center */}
          <TiltCard
            className="absolute top-[2%] left-[12%] w-[62%] z-10"
            style={{ rotate: "-2deg" }}
            onClick={() => document.getElementById("tour-gui")?.scrollIntoView({ behavior: "smooth", block: "start" })}
          >
            <span className="absolute top-2.5 left-3 z-10 px-2 py-0.5 rounded-full bg-[rgba(7,17,31,0.85)] border border-line text-cyan font-[family-name:var(--font-mono)] text-[0.7rem]">
              tu gui
            </span>
            <img src="/assets/media/gui-demo.png" alt="tokenusage GUI dashboard" className="w-full block" />
          </TiltCard>

          {/* tu — basic CLI, top-right */}
          <TiltCard
            className="absolute top-0 right-[0%] w-[46%] z-20"
            style={{ rotate: "2.5deg" }}
            onClick={() => document.getElementById("tour-daily")?.scrollIntoView({ behavior: "smooth", block: "start" })}
          >
            <span className="absolute top-2.5 left-3 z-10 px-2 py-0.5 rounded-full bg-[rgba(7,17,31,0.85)] border border-line text-cyan font-[family-name:var(--font-mono)] text-[0.7rem]">
              tu
            </span>
            <img src="/assets/media/cli-demo-padded.png" alt="tokenusage CLI report" className="w-full block" />
          </TiltCard>

          {/* tu img week — tall portrait, above tu live, shifted right */}
          <TiltCard
            className="absolute bottom-[12%] left-[22%] w-[25%] z-45"
            style={{ rotate: "-3deg" }}
            onClick={() => document.getElementById("tour-img")?.scrollIntoView({ behavior: "smooth", block: "start" })}
          >
            <span className="absolute top-2.5 left-3 z-10 px-2 py-0.5 rounded-full bg-[rgba(7,17,31,0.85)] border border-line text-cyan font-[family-name:var(--font-mono)] text-[0.7rem]">
              tu img week
            </span>
            <img src="/assets/media/share-week-demo.png" alt="tokenusage weekly share card" className="w-full block" />
          </TiltCard>

          {/* tu img day — bottom-right, overlapping */}
          <TiltCard
            className="absolute bottom-[24%] right-[2%] w-[40%] z-25"
            style={{ rotate: "-1.5deg" }}
            onClick={() => document.getElementById("tour-img")?.scrollIntoView({ behavior: "smooth", block: "start" })}
          >
            <span className="absolute top-2.5 left-3 z-10 px-2 py-0.5 rounded-full bg-[rgba(7,17,31,0.85)] border border-line text-cyan font-[family-name:var(--font-mono)] text-[0.7rem]">
              tu img day
            </span>
            <img src="/assets/media/share-demo.png" alt="tokenusage share card" className="w-full block" />
          </TiltCard>

          {/* tu live — wide bar, bottom spanning */}
          <TiltCard
            className="absolute bottom-0 left-[14%] w-[72%] z-40"
            style={{ rotate: "1deg" }}
            onClick={() => document.getElementById("tour-live")?.scrollIntoView({ behavior: "smooth", block: "start" })}
          >
            <span className="absolute top-2.5 left-3 z-10 px-2 py-0.5 rounded-full bg-[rgba(7,17,31,0.85)] border border-line text-cyan font-[family-name:var(--font-mono)] text-[0.7rem]">
              tu live
            </span>
            <img src="/assets/media/live-demo.png" alt="tokenusage live monitor" className="w-full block" />
          </TiltCard>
        </div>
      </div>
      </div>

      {/* Partners + Scroll indicator — at bottom of viewport */}
      <div className="mt-auto flex flex-col items-center gap-2 pt-3">
        <PartnerStrip />
        <ScrollIndicator />
      </div>
    </section>
  );
}
