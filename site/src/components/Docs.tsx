import { motion } from "framer-motion";
import { ExternalLink } from "lucide-react";
import { useI18n } from "../i18n";

const DOC_LINKS = [
  { href: "https://github.com/hanbu97/tokenusage#quick-start" },
  { href: "https://github.com/hanbu97/tokenusage/blob/main/docs/compare/tokenusage-vs-ccusage.md" },
  { href: "https://crates.io/crates/tokenusage" },
  { href: "https://www.npmjs.com/package/tokenusage" },
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
  const { messages } = useI18n();

  return (
    <section id="docs" className="mx-auto max-w-[min(1280px,calc(100vw-48px))] py-14">
      <motion.div
        className="max-w-[760px] mb-7"
        initial={{ opacity: 0, y: 20 }}
        whileInView={{ opacity: 1, y: 0 }}
        viewport={{ once: true, margin: "-10%" }}
        transition={{ duration: 0.5 }}
      >
        <span className="theme-badge inline-block mb-3.5 px-3 py-2 rounded-full border border-cyan/28 font-[family-name:var(--font-display)] text-[0.7rem] tracking-[0.12em] uppercase text-cyan">
          {messages.docs.badge}
        </span>
        <h2 className="mt-0 font-[family-name:var(--font-display)] text-[clamp(1.5rem,3vw,2.4rem)] leading-tight tracking-[0.04em]">
          {messages.docs.title}
        </h2>
        <p className="mt-4 text-text-soft leading-relaxed">
          {messages.docs.description}
        </p>
      </motion.div>

      <motion.div
        className="grid grid-cols-1 gap-5.5 sm:grid-cols-2"
        variants={container}
        initial="hidden"
        whileInView="show"
        viewport={{ once: true, margin: "-5%" }}
      >
        {DOC_LINKS.map((doc, index) => (
          <motion.a
            key={doc.href}
            href={doc.href}
            target="_blank"
            rel="noreferrer"
            className="glass p-5.5 group transition-all duration-200 hover:-translate-y-0.5 hover:border-cyan/30"
            variants={item}
          >
            <div className="flex items-center justify-between mb-2">
              <span className="text-text-dim font-[family-name:var(--font-display)] text-[0.7rem] tracking-[0.12em] uppercase">
                {messages.docs.links[index].label}
              </span>
              <ExternalLink size={14} className="text-text-dim opacity-0 transition-opacity group-hover:opacity-100" />
            </div>
            <h3 className="m-0 mb-3 font-[family-name:var(--font-display)] text-2xl">
              {messages.docs.links[index].title}
            </h3>
            <p className="m-0 text-text-soft leading-relaxed">{messages.docs.links[index].desc}</p>
          </motion.a>
        ))}
      </motion.div>
    </section>
  );
}
