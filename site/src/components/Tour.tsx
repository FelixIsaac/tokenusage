import { useState, useEffect, useRef, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Copy, Check } from "lucide-react";
import CastPlayer from "./CastPlayer";
import { useI18n } from "../i18n";

type TourPanelKey =
  | "daily"
  | "live"
  | "top"
  | "today"
  | "activity"
  | "heartbeat"
  | "img"
  | "gui"
  | "periods"
  | "statusline";

type TourImageAltKey = "dailyShare" | "weeklyShare" | "gui";

function CmdCopy({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  const { messages } = useI18n();

  const copy = useCallback(async (e: React.MouseEvent<HTMLButtonElement>) => {
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
    <button
      type="button"
      onClick={copy}
      aria-label={copied ? messages.common.copied : messages.common.copy}
      title={copied ? messages.common.copied : messages.common.copy}
      className="theme-copy-button inline-flex items-center justify-center w-5 h-5 rounded-md border text-text-dim cursor-pointer transition-all hover:border-cyan/30 hover:text-text-soft"
    >
      {copied ? <Check size={10} /> : <Copy size={10} />}
    </button>
  );
}

const LEFT_PANELS: Record<TourPanelKey, {
  link: string;
  img?: string;
  altKey?: TourImageAltKey;
  cast?: string;
  images?: { src: string; altKey: TourImageAltKey }[];
  commands?: string[];
}> = {
  daily: { link: "https://github.com/hanbu97/tokenusage#quick-start", cast: "/assets/casts/tu-daily.cast" },
  live: { link: "https://github.com/hanbu97/tokenusage#quick-start", cast: "/assets/casts/tu-live.cast" },
  top: { link: "https://github.com/hanbu97/tokenusage#quick-start", cast: "/assets/casts/tu-top.cast" },
  today: { link: "https://github.com/hanbu97/tokenusage#quick-start", cast: "/assets/casts/tu-today.cast" },
  activity: { link: "https://github.com/hanbu97/tokenusage#how-does---with-activity-work", cast: "/assets/casts/tu-activity.cast" },
  heartbeat: { link: "https://github.com/hanbu97/tokenusage#quick-start", cast: "/assets/casts/tu-heartbeat.cast" },
  img: {
    link: "https://github.com/hanbu97/tokenusage#quick-start",
    images: [
      { src: "/assets/media/share-demo.png", altKey: "dailyShare" },
      { src: "/assets/media/share-week-demo.png", altKey: "weeklyShare" },
    ],
  },
  gui: { link: "https://github.com/hanbu97/tokenusage#quick-start", img: "/assets/media/gui-demo.png", altKey: "gui" },
  periods: { link: "https://github.com/hanbu97/tokenusage#quick-start", cast: "/assets/casts/tu-weekly.cast" },
  statusline: {
    link: "https://github.com/hanbu97/tokenusage#quick-start",
    commands: ["tu statusline", "tu statusline --visual-burn-rate emoji"],
  },
};

const STEPS = [
  { key: "daily", index: "01", cmd: "tu" },
  { key: "live", index: "02", cmd: "tu live" },
  { key: "top", index: "03", cmd: "tu top" },
  { key: "today", index: "04", cmd: "tu today" },
  { key: "activity", index: "05", cmd: "tu activity" },
  { key: "heartbeat", index: "06", cmd: "tu heartbeat" },
  { key: "img", index: "07", cmd: "tu img" },
  { key: "gui", index: "08", cmd: "tu gui" },
  { key: "periods", index: "09", cmd: "tu weekly" },
  { key: "statusline", index: "10", cmd: "tu statusline" },
] as const satisfies { key: TourPanelKey; index: string; cmd: string }[];

export default function Tour() {
  const [active, setActive] = useState<TourPanelKey>("daily");
  const [preview, setPreview] = useState<{ src: string; alt: string } | null>(null);
  const sectionRef = useRef<HTMLElement>(null);
  const stepRefs = useRef<(HTMLDivElement | null)[]>([]);
  const manualRef = useRef(false);
  const manualTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const { messages } = useI18n();

  const handleClick = useCallback((key: TourPanelKey) => {
    setActive(key);
    manualRef.current = true;
    if (manualTimer.current) clearTimeout(manualTimer.current);
    manualTimer.current = setTimeout(() => {
      manualRef.current = false;
    }, 1500);
    const idx = STEPS.findIndex((step) => step.key === key);
    const el = stepRefs.current[idx];
    if (el) el.scrollIntoView({ behavior: "smooth", block: "start" });
  }, []);

  useEffect(() => {
    const keys = STEPS.map((step) => step.key);
    let ticking = false;

    const onScroll = () => {
      if (ticking || manualRef.current) return;
      ticking = true;
      requestAnimationFrame(() => {
        const line = window.innerHeight * 0.3;
        let bestIdx = 0;
        stepRefs.current.forEach((el, i) => {
          if (!el) return;
          if (el.getBoundingClientRect().top <= line) bestIdx = i;
        });
        setActive(keys[bestIdx]);
        ticking = false;
      });
    };

    window.addEventListener("scroll", onScroll, { passive: true });
    onScroll();
    return () => window.removeEventListener("scroll", onScroll);
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
        <span className="theme-badge inline-block mb-3.5 px-3 py-2 rounded-full border border-cyan/28 font-[family-name:var(--font-display)] text-[0.7rem] tracking-[0.12em] uppercase text-cyan">
          {messages.tour.badge}
        </span>
        <h2 className="mt-0 font-[family-name:var(--font-display)] text-[clamp(1.8rem,3.5vw,2.8rem)] leading-tight">
          {messages.tour.title}
        </h2>
        <p className="mt-4 text-text-soft leading-relaxed">
          {messages.tour.description}
        </p>
      </motion.div>

      <div className="grid items-start gap-5.5 lg:grid-cols-[minmax(0,1.15fr)_minmax(280px,0.7fr)]">
        <div className="grid gap-6">
          {STEPS.map((step, i) => {
            const panel = LEFT_PANELS[step.key];
            const panelCopy = messages.tour.panels[step.key];

            return (
              <div
                key={step.key}
                id={`tour-${step.key}`}
                ref={(el) => {
                  stepRefs.current[i] = el;
                }}
                className="glass min-h-[380px] p-4.5 lg:min-h-[440px] transition-opacity duration-300 scroll-mt-24 cursor-pointer"
                style={{ opacity: active === step.key ? 1 : 0.4 }}
                onClick={() => {
                  if (active !== step.key) handleClick(step.key);
                }}
              >
                <div className="flex items-center justify-between gap-5 mb-4">
                  <span className="font-[family-name:var(--font-display)] text-lg">{panelCopy.panelTitle}</span>
                  <a
                    href={panel.link}
                    target="_blank"
                    rel="noreferrer"
                    className="text-text-dim text-[0.92rem] hover:text-text-soft transition-colors"
                  >
                    {panelCopy.linkLabel}
                  </a>
                </div>

                {panel.cast && (
                  <CastPlayer
                    src={panel.cast}
                    active={active === step.key}
                    onRequestActivate={() => handleClick(step.key)}
                    className="theme-cast-frame w-full h-[380px] rounded-2xl border"
                  />
                )}

                {panel.img && (
                  <button
                    type="button"
                    onClick={() =>
                      setPreview({
                        src: panel.img!,
                        alt: messages.tour.imageAlts[panel.altKey!],
                      })
                    }
                    className="theme-media-frame w-full h-[calc(100%-58px)] rounded-2xl border overflow-hidden cursor-pointer transition-all hover:border-cyan/30 hover:scale-[1.01] p-0"
                  >
                    <img
                      src={panel.img}
                      alt={messages.tour.imageAlts[panel.altKey!]}
                      className="w-full h-full object-contain"
                    />
                  </button>
                )}

                {panel.images && (
                  <div className="grid grid-cols-2 gap-3 h-[380px]">
                    {panel.images.map((img) => (
                      <button
                        key={img.src}
                        type="button"
                        onClick={() =>
                          setPreview({
                            src: img.src,
                            alt: messages.tour.imageAlts[img.altKey],
                          })
                        }
                        className="theme-media-frame w-full h-full rounded-2xl border overflow-hidden cursor-pointer transition-all hover:border-cyan/30 hover:scale-[1.01] p-0"
                      >
                        <img
                          src={img.src}
                          alt={messages.tour.imageAlts[img.altKey]}
                          className="w-full h-full object-contain"
                        />
                      </button>
                    ))}
                  </div>
                )}

                {panel.commands && "contentHeading" in panelCopy && "contentBody" in panelCopy && (
                  <div className="grid gap-4.5 h-[calc(100%-58px)] content-center">
                    <div className="theme-soft-panel p-6 rounded-[18px] border border-line">
                      <h3 className="m-0 mb-3 font-[family-name:var(--font-display)] text-2xl">
                        {panelCopy.contentHeading}
                      </h3>
                      <p className="text-text-soft leading-relaxed">{panelCopy.contentBody}</p>
                    </div>
                    <div className="flex flex-wrap gap-2.5">
                      {panel.commands.map((command) => (
                        <code
                          key={command}
                          className="theme-code-chip px-2.5 py-2.5 rounded-xl border text-text-soft"
                        >
                          {command}
                        </code>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            );
          })}
        </div>

        <div className="sticky top-20 grid gap-2">
          {STEPS.map((step) => (
            <div
              key={step.key}
              role="button"
              tabIndex={0}
              aria-pressed={active === step.key}
              data-active={active === step.key}
              onClick={() => handleClick(step.key)}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  handleClick(step.key);
                }
              }}
              className="theme-step tour-step grid grid-cols-[36px_minmax(0,1fr)] gap-2.5 px-3 py-2.5 rounded-xl border text-left cursor-pointer transition-all duration-300"
            >
              <div className="theme-step-index inline-flex items-center justify-center w-[36px] h-[36px] rounded-[10px] border text-cyan font-[family-name:var(--font-display)] text-xs">
                {step.index}
              </div>
              <div className="flex flex-col justify-center gap-1">
                <div className="inline-flex items-center gap-1.5 self-start">
                  <code className="theme-code-chip px-1.5 py-0.5 rounded-md border text-cyan text-[0.7rem]">
                    {step.cmd}
                  </code>
                  <CmdCopy text={step.cmd} />
                </div>
                <h3 className="m-0 text-[0.84rem] leading-snug font-medium">
                  {messages.tour.panels[step.key].stepTitle}
                </h3>
              </div>
            </div>
          ))}
        </div>
      </div>

      <AnimatePresence>
        {preview && (
          <motion.div
            className="theme-modal-overlay fixed inset-0 z-50 flex items-center justify-center backdrop-blur-sm cursor-pointer"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
            onClick={() => setPreview(null)}
          >
            <div className="relative cursor-default" onClick={(e) => e.stopPropagation()}>
              <motion.img
                src={preview.src}
                alt={preview.alt}
                className="theme-preview-image max-w-[90vw] max-h-[85vh] rounded-2xl border shadow-2xl"
                initial={{ scale: 0.9, opacity: 0 }}
                animate={{ scale: 1, opacity: 1 }}
                exit={{ scale: 0.9, opacity: 0 }}
                transition={{ duration: 0.2 }}
              />
              <button
                type="button"
                onClick={() => setPreview(null)}
                className="theme-modal-close absolute -top-3 -right-3 w-8 h-8 rounded-full border text-text-soft flex items-center justify-center cursor-pointer transition-colors"
              >
                ✕
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </section>
  );
}
