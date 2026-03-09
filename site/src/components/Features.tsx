import { motion } from "framer-motion";


const FEATURES = [
  {
    cmd: "tu",
    tourKey: "daily",
    meta: "Daily, weekly, monthly",
    title: "Fast merged token reports",
    desc: "Keep the classic token report by default. Add activity context only when you want it.",
    img: "/assets/media/cli-demo-padded.png",
    alt: "tokenusage cli report",
  },
  {
    cmd: "tu live",
    tourKey: "live",
    meta: "Official limits + forecast",
    title: "See limits before they hurt",
    desc: "Watch 5h and weekly windows, current burn, projected end, and official limit state in one view.",
    img: "/assets/media/live-demo.png",
    alt: "tokenusage live view",
  },
  {
    cmd: "tu today",
    tourKey: "today",
    meta: "Time and efficiency",
    title: "Activity views with heartbeat support",
    desc: "Infer coding time locally from AI usage events, or tighten it with the native heartbeat collector.",
    tags: ["coding time", "tokens / hour", "cost / hour", "project breakdown"],
  },
  {
    cmd: "tu gui",
    tourKey: "gui",
    meta: "Desktop visibility",
    title: "Desktop dashboard with charts",
    desc: "Full-screen dashboard for scrolling reports, trend charts, filters, and visual summaries.",
    img: "/assets/media/gui-demo.png",
    alt: "tokenusage gui dashboard",
  },
];

const container = {
  hidden: {},
  show: { transition: { staggerChildren: 0.12 } },
};

const item = {
  hidden: { opacity: 0, y: 28 },
  show: { opacity: 1, y: 0, transition: { duration: 0.5, ease: "easeOut" } },
};

export default function Features() {
  return (
    <section id="product" className="mx-auto max-w-[min(1280px,calc(100vw-48px))] py-14">
      <motion.div
        className="max-w-[760px] mb-7"
        initial={{ opacity: 0, y: 20 }}
        whileInView={{ opacity: 1, y: 0 }}
        viewport={{ once: true, margin: "-10%" }}
        transition={{ duration: 0.5 }}
      >
        <span className="inline-block mb-3.5 px-3 py-2 rounded-full border border-cyan/28 bg-[rgba(9,23,40,0.7)] font-[family-name:var(--font-display)] text-[0.7rem] tracking-[0.12em] uppercase text-cyan">
          Product
        </span>
        <h2 className="mt-0 font-[family-name:var(--font-display)] text-[clamp(2rem,4vw,3.5rem)] leading-none">
          Designed for the real workflow, not just a single terminal command.
        </h2>
        <p className="mt-4 text-text-soft leading-relaxed">
          The website can explain what the README cannot show cleanly: how the CLI, TUI,
          GUI, activity views, and share cards fit together as one product.
        </p>
      </motion.div>

      <motion.div
        className="grid grid-cols-1 gap-5.5 md:grid-cols-2"
        variants={container}
        initial="hidden"
        whileInView="show"
        viewport={{ once: true, margin: "-5%" }}
      >
        {FEATURES.map((f) => (
          <motion.article
            key={f.cmd}
            className="glass p-5.5 group cursor-pointer"
            variants={item}
            onClick={() => {
              document.getElementById(`tour-${f.tourKey}`)?.scrollIntoView({ behavior: "smooth", block: "start" });
            }}
            whileHover={{ scale: 1.01 }}
            whileTap={{ scale: 0.99 }}
          >
            <div className="flex items-center justify-between gap-4.5 mb-3.5 text-text-dim text-[0.86rem]">
              <code className="px-2.5 py-2 rounded-[10px] bg-[rgba(8,17,30,0.82)] border border-[rgba(116,150,198,0.2)] text-cyan">
                {f.cmd}
              </code>
              <span>{f.meta}</span>
            </div>
            <h3 className="m-0 mb-3 font-[family-name:var(--font-display)] text-2xl">{f.title}</h3>
            <p className="text-text-soft leading-relaxed">{f.desc}</p>
            {f.img && (
              <img
                src={f.img}
                alt={f.alt}
                className="mt-4.5 w-full rounded-[18px] border border-[rgba(101,122,165,0.18)] transition-transform duration-300 group-hover:scale-[1.02]"
              />
            )}
            {f.tags && (
              <div className="mt-4 flex flex-wrap gap-2.5">
                {f.tags.map((t) => (
                  <span
                    key={t}
                    className="px-3 py-2.5 rounded-xl border border-[rgba(111,145,194,0.18)] bg-[rgba(8,18,31,0.82)] text-text-soft text-sm"
                  >
                    {t}
                  </span>
                ))}
              </div>
            )}
          </motion.article>
        ))}
      </motion.div>
    </section>
  );
}
