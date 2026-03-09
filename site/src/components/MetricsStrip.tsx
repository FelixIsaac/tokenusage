import { motion } from "framer-motion";

const METRICS = [
  { value: "0.08s", label: "Warm run on 1,521-file, 2.2 GB Claude log set" },
  { value: "214x", label: "0.08s vs 17.15s — Rust + parallel scan + caching" },
  { value: "3 sources", label: "Codex, Claude, and Antigravity in one merged view" },
  { value: "100% local", label: "Logs stay on your machine. Only pricing metadata is fetched." },
];

const container = {
  hidden: {},
  show: { transition: { staggerChildren: 0.1 } },
};

const item = {
  hidden: { opacity: 0, y: 24 },
  show: { opacity: 1, y: 0, transition: { duration: 0.5, ease: "easeOut" as const } },
};

export default function MetricsStrip() {
  return (
    <motion.section
      className="mx-auto grid max-w-[min(1280px,calc(100vw-48px))] grid-cols-1 gap-5 pt-3 pb-14 sm:grid-cols-2 lg:grid-cols-4"
      variants={container}
      initial="hidden"
      whileInView="show"
      viewport={{ once: true, margin: "-10%" }}
    >
      {METRICS.map((m) => (
        <motion.article
          key={m.value}
          className="glass flex min-h-[150px] flex-col items-start justify-between gap-2.5 p-5.5"
          variants={item}
        >
          <div className="font-[family-name:var(--font-accent)] text-[clamp(1.8rem,3vw,2.6rem)] text-cyan">
            {m.value}
          </div>
          <div className="text-text-soft text-sm uppercase tracking-[0.12em]">{m.label}</div>
        </motion.article>
      ))}
    </motion.section>
  );
}
