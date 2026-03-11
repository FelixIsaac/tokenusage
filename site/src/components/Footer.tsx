import { motion } from "framer-motion";
import { Github } from "lucide-react";
import { useI18n } from "../i18n";

export default function Footer() {
  const { messages } = useI18n();

  return (
    <section className="mx-auto max-w-[min(1280px,calc(100vw-48px))] py-14">
      <motion.div
        className="glass flex flex-col items-start gap-8 p-7.5 sm:flex-row sm:items-center sm:justify-between"
        initial={{ opacity: 0, y: 20 }}
        whileInView={{ opacity: 1, y: 0 }}
        viewport={{ once: true, margin: "-10%" }}
        transition={{ duration: 0.5 }}
      >
        <div>
          <span className="inline-block mb-3.5 px-3 py-2 rounded-full border border-cyan/28 bg-[rgba(9,23,40,0.7)] font-[family-name:var(--font-display)] text-[0.7rem] tracking-[0.12em] uppercase text-cyan">
            {messages.footer.badge}
          </span>
          <h2 className="mt-4 mb-0 font-[family-name:var(--font-display)] text-[clamp(2rem,4vw,3.3rem)] leading-none">
            {messages.footer.title}
          </h2>
          <p className="mt-4 text-text-soft leading-relaxed">
            {messages.footer.description}
          </p>
        </div>

        <div className="flex flex-wrap gap-3">
          <a
            href="https://github.com/hanbu97/tokenusage"
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-2.5 rounded-full border border-white/12 bg-[linear-gradient(180deg,rgba(30,34,42,0.96),rgba(9,11,15,0.96))] px-5 py-3 font-[family-name:var(--font-display)] text-[0.8rem] tracking-wider text-[#f5f8fc] shadow-[0_14px_34px_rgba(0,0,0,0.36)] transition-all hover:-translate-y-0.5 hover:border-cyan/30"
          >
            <Github size={15} />
            {messages.footer.starOnGithub}
          </a>
          <a
            href="mailto:contact@tokenusage.org"
            className="inline-flex items-center rounded-full border border-[rgba(112,145,188,0.28)] bg-[rgba(10,22,38,0.62)] px-5 py-3 font-[family-name:var(--font-display)] text-[0.8rem] tracking-wider text-text-soft transition-all hover:-translate-y-0.5 hover:border-cyan/30"
          >
            {messages.footer.contact}
          </a>
        </div>
      </motion.div>

      <div className="mt-12 pb-8 text-center text-text-dim text-sm">
        {messages.footer.builtWith}
      </div>
    </section>
  );
}
