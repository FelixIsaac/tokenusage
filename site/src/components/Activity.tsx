import { motion } from "framer-motion";

const COMMANDS = `tu heartbeat watch .
tu heartbeat stats
tu today
tu activity --days 14`;

const UNLOCKS = [
  "Coding time", "Tok/hr", "Cost/hr",
  "Project share", "Source share", "Hourly windows",
];

export default function Activity() {
  return (
    <section id="activity" className="mx-auto grid max-w-[min(1280px,calc(100vw-48px))] items-stretch gap-6 py-14 lg:grid-cols-[minmax(0,1fr)_minmax(0,0.95fr)]">
      <motion.div
        initial={{ opacity: 0, y: 24 }}
        whileInView={{ opacity: 1, y: 0 }}
        viewport={{ once: true, margin: "-10%" }}
        transition={{ duration: 0.5 }}
      >
        <span className="inline-block mb-3.5 px-3 py-2 rounded-full border border-cyan/28 bg-[rgba(9,23,40,0.7)] font-[family-name:var(--font-display)] text-[0.7rem] tracking-[0.12em] uppercase text-cyan">
          Activity
        </span>
        <h2 className="mt-4.5 mb-0 font-[family-name:var(--font-display)] text-[clamp(2rem,4vw,3.3rem)] leading-none">
          From raw usage logs to a time view you can reason about.
        </h2>
        <p className="mt-4 text-text-soft leading-relaxed">
          WakaTime-style questions matter in AI coding too: how long were you active, which project consumed the most,
          and what did each hour cost? tokenusage now has a local-first answer.
        </p>
        <ul className="mt-6 flex flex-wrap gap-2.5 list-none p-0">
          {[
            "dedicated today and activity commands",
            "heartbeat-backed activity collection for stronger coverage",
            "project, source, model, token, and cost breakdowns",
            "opt-in activity columns on merged daily reports",
          ].map((item) => (
            <li
              key={item}
              className="px-3 py-2.5 rounded-xl border border-[rgba(111,145,194,0.18)] bg-[rgba(8,18,31,0.82)] text-text-soft text-sm"
            >
              {item}
            </li>
          ))}
        </ul>
      </motion.div>

      <motion.div
        className="glass p-5.5"
        initial={{ opacity: 0, y: 24 }}
        whileInView={{ opacity: 1, y: 0 }}
        viewport={{ once: true, margin: "-10%" }}
        transition={{ duration: 0.5, delay: 0.15 }}
      >
        <div className="grid h-full gap-4">
          <div className="p-5.5 rounded-[18px] bg-[rgba(8,17,31,0.72)] border border-[rgba(113,143,187,0.2)]">
            <span className="block mb-3 text-text-dim font-[family-name:var(--font-display)] text-[0.7rem] tracking-[0.12em] uppercase">
              Heartbeat commands
            </span>
            <pre className="m-0 text-lime leading-[1.8] whitespace-pre-wrap"><code>{COMMANDS}</code></pre>
          </div>

          <div
            className="p-5.5 rounded-[18px] border border-[rgba(113,143,187,0.2)]"
            style={{
              background: "linear-gradient(180deg, rgba(14,27,46,0.94), rgba(7,18,31,0.92)), radial-gradient(circle at top right, rgba(112,245,191,0.12), transparent 52%)"
            }}
          >
            <span className="block mb-3 text-text-dim font-[family-name:var(--font-display)] text-[0.7rem] tracking-[0.12em] uppercase">
              What it unlocks
            </span>
            <div className="flex flex-wrap gap-2.5">
              {UNLOCKS.map((u) => (
                <span
                  key={u}
                  className="px-3 py-2.5 rounded-xl border border-[rgba(111,145,194,0.18)] bg-[rgba(8,18,31,0.82)] text-text-soft text-sm"
                >
                  {u}
                </span>
              ))}
            </div>
          </div>
        </div>
      </motion.div>
    </section>
  );
}
