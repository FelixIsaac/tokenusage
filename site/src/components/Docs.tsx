import { motion } from "framer-motion";
import { ExternalLink } from "lucide-react";

const DOC_LINKS = [
  {
    label: "Quick start",
    title: "Command-first onboarding",
    desc: "Install, run, go live, turn on activity, and generate share cards.",
    href: "https://github.com/hanbu97/tokenusage#quick-start",
  },
  {
    label: "Comparison",
    title: "tokenusage vs ccusage",
    desc: "Performance, merged-source reporting, live monitoring, and product surface area.",
    href: "https://github.com/hanbu97/tokenusage/blob/main/docs/compare/tokenusage-vs-ccusage.md",
  },
  {
    label: "Registry",
    title: "Rust crate",
    desc: "Install from crates.io or cargo-binstall and use tu locally.",
    href: "https://crates.io/crates/tokenusage",
  },
  {
    label: "Registry",
    title: "npm package",
    desc: "Global install path for users who want one-line onboarding.",
    href: "https://www.npmjs.com/package/tokenusage",
  },
];

const container = {
  hidden: {},
  show: { transition: { staggerChildren: 0.08 } },
};

const item = {
  hidden: { opacity: 0, y: 20 },
  show: { opacity: 1, y: 0, transition: { duration: 0.45, ease: "easeOut" as const } },
};

export default function Docs() {
  return (
    <section id="docs" className="mx-auto max-w-[min(1280px,calc(100vw-48px))] py-14">
      <motion.div
        className="max-w-[760px] mb-7"
        initial={{ opacity: 0, y: 20 }}
        whileInView={{ opacity: 1, y: 0 }}
        viewport={{ once: true, margin: "-10%" }}
        transition={{ duration: 0.5 }}
      >
        <span className="inline-block mb-3.5 px-3 py-2 rounded-full border border-cyan/28 bg-[rgba(9,23,40,0.7)] font-[family-name:var(--font-display)] text-[0.7rem] tracking-[0.12em] uppercase text-cyan">
          Docs
        </span>
        <h2 className="mt-0 font-[family-name:var(--font-display)] text-[clamp(1.5rem,3vw,2.4rem)] leading-tight tracking-[0.04em]">
          Docs, source, and registries.
        </h2>
        <p className="mt-4 text-text-soft leading-relaxed">
          Everything you need to get started, compare options, and install from your preferred registry.
        </p>
      </motion.div>

      <motion.div
        className="grid grid-cols-1 gap-5.5 sm:grid-cols-2"
        variants={container}
        initial="hidden"
        whileInView="show"
        viewport={{ once: true, margin: "-5%" }}
      >
        {DOC_LINKS.map((doc) => (
          <motion.a
            key={doc.title}
            href={doc.href}
            target="_blank"
            rel="noreferrer"
            className="glass p-5.5 group transition-all duration-200 hover:-translate-y-0.5 hover:border-cyan/30"
            variants={item}
          >
            <div className="flex items-center justify-between mb-2">
              <span className="text-text-dim font-[family-name:var(--font-display)] text-[0.7rem] tracking-[0.12em] uppercase">
                {doc.label}
              </span>
              <ExternalLink size={14} className="text-text-dim opacity-0 transition-opacity group-hover:opacity-100" />
            </div>
            <h3 className="m-0 mb-3 font-[family-name:var(--font-display)] text-2xl">{doc.title}</h3>
            <p className="m-0 text-text-soft leading-relaxed">{doc.desc}</p>
          </motion.a>
        ))}
      </motion.div>
    </section>
  );
}
