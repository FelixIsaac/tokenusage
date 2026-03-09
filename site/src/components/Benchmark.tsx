import { motion } from "framer-motion";

const BENCHMARKS = [
  {
    kicker: "Claude warm run",
    value: "214x faster",
    desc: "0.08s vs 17.15s on a 1,521-file, 2.2 GB log set.",
  },
  {
    kicker: "Codex warm run",
    value: "138x faster",
    desc: "0.15s vs 20.76s on a 91-file, 1.7 GB log set.",
  },
  {
    kicker: "Why",
    value: "Rust + parallel scan + caching",
    desc: "Local-first architecture focused on repeated everyday use, not only one-off reports.",
  },
];

const container = {
  hidden: {},
  show: { transition: { staggerChildren: 0.1 } },
};

const item = {
  hidden: { opacity: 0, y: 24 },
  show: { opacity: 1, y: 0, transition: { duration: 0.5, ease: "easeOut" } },
};

export default function Benchmark() {
  return (
    <section id="benchmark" className="mx-auto max-w-[min(1280px,calc(100vw-48px))] py-14">
      <motion.div
        className="max-w-[760px] mb-7"
        initial={{ opacity: 0, y: 20 }}
        whileInView={{ opacity: 1, y: 0 }}
        viewport={{ once: true, margin: "-10%" }}
        transition={{ duration: 0.5 }}
      >
        <span className="inline-block mb-3.5 px-3 py-2 rounded-full border border-cyan/28 bg-[rgba(9,23,40,0.7)] font-[family-name:var(--font-display)] text-[0.7rem] tracking-[0.12em] uppercase text-cyan">
          Why switch
        </span>
        <h2 className="mt-0 font-[family-name:var(--font-display)] text-[clamp(2rem,4vw,3.5rem)] leading-none">
          Faster than ccusage, broader than ccusage.
        </h2>
        <p className="mt-4 text-text-soft leading-relaxed">
          The performance story and the product story matter together: fast parsing, merged multi-source reporting,
          official live limits, activity views, GUI, and share cards.
        </p>
      </motion.div>

      <motion.div
        className="grid grid-cols-1 gap-5.5 md:grid-cols-3"
        variants={container}
        initial="hidden"
        whileInView="show"
        viewport={{ once: true, margin: "-5%" }}
      >
        {BENCHMARKS.map((b) => (
          <motion.article key={b.kicker} className="glass p-5.5" variants={item}>
            <span className="text-text-dim font-[family-name:var(--font-display)] text-[0.7rem] tracking-[0.12em] uppercase">
              {b.kicker}
            </span>
            <strong className="block mt-3 font-[family-name:var(--font-display)] text-[clamp(1.8rem,3vw,2.7rem)] text-gold">
              {b.value}
            </strong>
            <p className="mt-3 text-text-soft leading-relaxed">{b.desc}</p>
          </motion.article>
        ))}
      </motion.div>
    </section>
  );
}
