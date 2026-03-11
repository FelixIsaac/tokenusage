import { useState, useCallback, type ReactNode } from "react";
import { motion } from "framer-motion";
import { Copy, Check, Package } from "lucide-react";
import { useI18n } from "../i18n";

function NpmIcon({ size = 14 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor">
      <path d="M0 7.334v8h6.666v1.332H12v-1.332h12v-8H0zm6.666 6.664H5.334v-4H3.999v4H1.335V8.667h5.331v5.331zm4 0v1.336H8.001V8.667h5.334v5.332h-2.669zm12.001 0h-1.33v-4h-1.336v4h-1.335v-4h-1.33v4h-2.671V8.667h8.002v5.331zM10.665 10H12v2.667h-1.335V10z" />
    </svg>
  );
}

function PythonIcon({ size = 14 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor">
      <path d="M14.25.18l.9.2.73.26.59.3.45.32.34.34.25.34.16.33.1.3.04.26.02.2-.01.13V8.5l-.05.63-.13.55-.21.46-.26.38-.3.31-.33.25-.35.19-.35.14-.33.1-.3.07-.26.04-.21.02H8.77l-.69.05-.59.14-.5.22-.41.27-.33.32-.27.35-.2.36-.15.37-.1.35-.07.32-.04.27-.02.21v3.68H3.21l-.33-.12-.3-.17-.27-.21-.23-.25-.19-.27-.16-.3-.13-.31-.1-.32-.07-.32-.05-.3-.03-.26-.01-.23.01-4.95.04-.52.09-.48.13-.43.16-.38.19-.33.22-.28.24-.24.26-.2.27-.17.28-.14.28-.11.27-.09.26-.07.24-.06.21-.04.18-.03h7.67l.58-.06.47-.15.38-.22.3-.26.24-.29.19-.31.14-.32.1-.32.06-.3.04-.26.02-.18V.5l-.01-.13-.03-.2-.06-.26-.09-.3-.14-.33-.19-.34-.25-.34-.33-.34-.43-.32-.55-.3-.69-.26-.84-.2-1-.13-1.17-.06L12 0c-1.48 0-2.63.08-3.45.24zm-1.45 2.27a.96.96 0 1 1 0 1.92.96.96 0 0 1 0-1.92zM9.75 23.82l-.9-.2-.73-.26-.59-.3-.45-.32-.34-.34-.25-.34-.16-.33-.1-.3-.04-.26-.02-.2v-.13l.01-5.62.05-.63.13-.55.21-.46.26-.38.3-.31.33-.25.35-.19.35-.14.33-.1.3-.07.26-.04.21-.02h5.25l.69-.05.59-.14.5-.22.41-.27.33-.32.27-.35.2-.36.15-.37.1-.35.07-.32.04-.27.02-.21V7.5h4.43l.33.12.3.17.27.21.23.25.19.27.16.3.13.31.1.32.07.32.05.3.03.26.01.23v4.95l-.04.52-.09.48-.13.43-.16.38-.19.33-.22.28-.24.24-.26.2-.27.17-.28.14-.28.11-.27.09-.26.07-.24.06-.21.04-.18.03H9.42l-.58.06-.47.15-.38.22-.3.26-.24.29-.19.31-.14.32-.1.32-.06.3-.04.26-.02.18v3.82l.01.13.03.2.06.26.09.3.14.33.19.34.25.34.33.34.43.32.55.3.69.26.84.2 1 .13 1.17.06h.77c1.48 0 2.63-.08 3.45-.24zm1.45-2.27a.96.96 0 1 1 0-1.92.96.96 0 0 1 0 1.92z" />
    </svg>
  );
}

const SOURCES: { key: string; label: string; icon: ReactNode; install: string }[] = [
  {
    key: "npm",
    label: "npm",
    icon: <NpmIcon />,
    install: "npm install -g tokenusage\nnpx tokenusage",
  },
  {
    key: "cargo",
    label: "cargo",
    icon: <Package size={14} />,
    install: "cargo install tokenusage --bin tu\ncargo binstall tokenusage --no-confirm",
  },
  {
    key: "pip",
    label: "pip",
    icon: <PythonIcon />,
    install: "pip install tokenusage",
  },
];

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  const { messages } = useI18n();

  const copy = useCallback(async () => {
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
      onClick={copy}
      aria-label={copied ? messages.common.copied : messages.common.copy}
      title={copied ? messages.common.copied : messages.common.copy}
      className="theme-copy-button shrink-0 flex items-center gap-1.5 px-2.5 py-1 rounded-lg border text-text-dim text-[0.7rem] cursor-pointer transition-all hover:border-cyan/30 hover:text-text-soft"
    >
      {copied ? <Check size={12} /> : <Copy size={12} />}
    </button>
  );
}

export default function Install() {
  const [active, setActive] = useState("npm");
  const { messages } = useI18n();

  const source = SOURCES.find((s) => s.key === active)!;
  const lines = source.install.split("\n");

  return (
    <section id="install" className="mx-auto max-w-[min(1280px,calc(100vw-48px))] py-14">
      <motion.div
        className="max-w-[760px] mb-7"
        initial={{ opacity: 0, y: 20 }}
        whileInView={{ opacity: 1, y: 0 }}
        viewport={{ once: true, margin: "-10%" }}
        transition={{ duration: 0.5 }}
      >
        <span className="theme-badge inline-block mb-3.5 px-3 py-2 rounded-full border border-cyan/28 font-[family-name:var(--font-display)] text-[0.7rem] tracking-[0.12em] uppercase text-cyan">
          {messages.install.badge}
        </span>
        <h2 className="mt-0 font-[family-name:var(--font-display)] text-[clamp(1.5rem,3vw,2.4rem)] leading-tight tracking-[0.04em]">
          {messages.install.title}
        </h2>
      </motion.div>

      <motion.div
        className="theme-install-card glass w-full p-5.5"
        initial={{ opacity: 0, y: 24 }}
        whileInView={{ opacity: 1, y: 0 }}
        viewport={{ once: true, margin: "-5%" }}
        transition={{ duration: 0.5 }}
      >
        <div className="flex flex-wrap gap-2 mb-5">
          {SOURCES.map((sourceOption) => (
            <button
              key={sourceOption.key}
              onClick={() => setActive(sourceOption.key)}
              data-active={active === sourceOption.key}
              className="theme-pill-tab inline-flex items-center gap-2 min-h-[33px] px-3.5 rounded-full border font-[family-name:var(--font-display)] text-[0.76rem] tracking-wider cursor-pointer transition-all"
            >
              {sourceOption.icon}
              {sourceOption.label}
            </button>
          ))}
        </div>

        <div className="flex flex-col gap-2.5 mb-5">
          {lines.map((line) => (
            <div
              key={line}
              className="theme-command-row flex items-center gap-3 px-4 py-3 rounded-2xl border transition-colors hover:border-cyan/20"
            >
              <code className="theme-command-text flex-1 text-[0.9rem] leading-relaxed">{line}</code>
              <CopyButton text={line} />
            </div>
          ))}
        </div>

        <p className="m-0 mb-3 text-text-dim text-[0.72rem] font-[family-name:var(--font-display)] tracking-[0.12em] uppercase">
          {messages.install.thenRun}
        </p>
        <div className="flex flex-wrap gap-2.5">
          {messages.install.commands.map((command) => (
            <code
              key={command.cmd}
              className="theme-code-chip group relative px-3 py-2 rounded-xl border text-lime text-sm cursor-default transition-colors hover:border-cyan/25"
              title={command.tip}
            >
              {command.cmd}
              <span className="theme-tooltip pointer-events-none absolute bottom-full left-1/2 -translate-x-1/2 mb-2 px-2.5 py-1.5 rounded-lg border border-line-strong/30 text-text-soft text-[0.75rem] whitespace-nowrap opacity-0 scale-95 transition-all group-hover:opacity-100 group-hover:scale-100">
                {command.tip}
              </span>
            </code>
          ))}
        </div>
      </motion.div>
    </section>
  );
}
