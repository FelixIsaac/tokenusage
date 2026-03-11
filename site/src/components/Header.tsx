import { useState, useEffect, useCallback } from "react";
import { Github } from "lucide-react";
import { LANGUAGE_OPTIONS, useI18n } from "../i18n";

export default function Header() {
  const [scrolled, setScrolled] = useState(false);
  const { locale, setLocale, messages } = useI18n();

  const scrollTo = useCallback((id: string) => {
    document.getElementById(id)?.scrollIntoView({ behavior: "smooth" });
  }, []);

  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 20);
    window.addEventListener("scroll", onScroll, { passive: true });
    return () => window.removeEventListener("scroll", onScroll);
  }, []);

  return (
    <header
      className={`sticky top-0 z-50 mx-auto flex max-w-[min(1280px,calc(100vw-48px))] items-center justify-between gap-6 rounded-full border px-4 py-1.5 backdrop-blur-xl transition-all duration-300 ${
        scrolled
          ? "border-line-strong/30 bg-bg/80 shadow-2xl"
          : "border-line bg-[rgba(8,17,31,0.74)] shadow-lg"
      }`}
    >
      <button
        onClick={() => window.scrollTo({ top: 0, behavior: "smooth" })}
        className="flex items-center rounded-full border border-white/10 bg-[rgba(8,18,33,0.46)] p-1 cursor-pointer"
        aria-label={messages.header.homeAria}
      >
        <img
          src="/assets/branding/tokenusage-logomark.svg"
          alt="tokenusage logo"
          className="h-8 w-8 drop-shadow-[0_0_18px_rgba(99,231,225,0.14)]"
        />
      </button>

      <nav className="hidden items-center gap-5 text-text-soft text-[0.95rem] md:flex" aria-label={messages.header.navAria}>
        {messages.header.nav.map((link) => (
          <button key={link.target} onClick={() => scrollTo(link.target)} className="cursor-pointer bg-transparent border-none text-inherit text-[0.95rem] transition-colors hover:text-text-primary">
            {link.label}
          </button>
        ))}
      </nav>

      <div className="flex items-center gap-2.5 ml-auto">
        <div
          className="inline-flex items-center gap-0.5 rounded-full border border-[rgba(112,145,188,0.2)] bg-[rgba(9,20,35,0.76)] p-1 shadow-[0_10px_24px_rgba(0,0,0,0.24)]"
          aria-label={messages.header.languageSwitcherAria}
          role="group"
        >
          {LANGUAGE_OPTIONS.map((option) => (
            <button
              key={option.code}
              type="button"
              onClick={() => setLocale(option.code)}
              lang={option.code}
              title={option.nativeName}
              aria-pressed={locale === option.code}
              className={`min-w-[2.1rem] rounded-full px-2.5 py-1 font-[family-name:var(--font-display)] text-[0.68rem] tracking-[0.12em] transition-all ${
                locale === option.code
                  ? "bg-[rgba(23,58,60,0.95)] text-cyan shadow-[0_0_0_1px_rgba(95,231,226,0.18),0_10px_24px_rgba(95,231,226,0.14)]"
                  : "text-text-dim hover:bg-[rgba(16,30,49,0.88)] hover:text-text-soft"
              }`}
            >
              {option.label}
            </button>
          ))}
        </div>
        <a
          href="https://github.com/hanbu97/tokenusage"
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-2 rounded-full border border-white/12 bg-[linear-gradient(180deg,rgba(30,34,42,0.96),rgba(9,11,15,0.96))] px-3.5 py-1.5 font-[family-name:var(--font-display)] text-[0.78rem] tracking-wider text-[#f5f8fc] shadow-[0_14px_34px_rgba(0,0,0,0.36)] transition-all hover:-translate-y-0.5 hover:border-cyan/30"
        >
          <Github size={15} />
          <span className="hidden sm:inline">{messages.header.github}</span>
        </a>
        <button
          onClick={() => scrollTo("install")}
          className="hidden rounded-full border border-[rgba(112,145,188,0.28)] bg-[rgba(10,22,38,0.62)] px-3.5 py-1.5 font-[family-name:var(--font-display)] text-[0.78rem] tracking-wider text-text-soft cursor-pointer transition-all hover:-translate-y-0.5 hover:border-cyan/30 sm:inline-flex"
        >
          {messages.header.install}
        </button>
      </div>
    </header>
  );
}
