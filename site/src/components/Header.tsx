import { useState, useEffect, useCallback } from "react";
import { Github } from "lucide-react";

const NAV_LINKS = [
  { label: "Tour", target: "tour" },
  { label: "Install", target: "install" },
  { label: "Docs", target: "docs" },
];

export default function Header() {
  const [scrolled, setScrolled] = useState(false);

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
        aria-label="tokenusage home"
      >
        <img
          src="/assets/branding/tokenusage-logomark.svg"
          alt="tokenusage logo"
          className="h-8 w-8 drop-shadow-[0_0_18px_rgba(99,231,225,0.14)]"
        />
      </button>

      <nav className="hidden items-center gap-5 text-text-soft text-[0.95rem] md:flex" aria-label="Primary">
        {NAV_LINKS.map((link) => (
          <button key={link.target} onClick={() => scrollTo(link.target)} className="cursor-pointer bg-transparent border-none text-inherit text-[0.95rem] transition-colors hover:text-text-primary">
            {link.label}
          </button>
        ))}
      </nav>

      <div className="flex items-center gap-3 ml-auto">
        <a
          href="https://github.com/hanbu97/tokenusage"
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-2 rounded-full border border-white/12 bg-[linear-gradient(180deg,rgba(30,34,42,0.96),rgba(9,11,15,0.96))] px-3.5 py-1.5 font-[family-name:var(--font-display)] text-[0.78rem] tracking-wider text-[#f5f8fc] shadow-[0_14px_34px_rgba(0,0,0,0.36)] transition-all hover:-translate-y-0.5 hover:border-cyan/30"
        >
          <Github size={15} />
          GitHub
        </a>
        <button
          onClick={() => scrollTo("install")}
          className="hidden rounded-full border border-[rgba(112,145,188,0.28)] bg-[rgba(10,22,38,0.62)] px-3.5 py-1.5 font-[family-name:var(--font-display)] text-[0.78rem] tracking-wider text-text-soft cursor-pointer transition-all hover:-translate-y-0.5 hover:border-cyan/30 sm:inline-flex"
        >
          Install
        </button>
      </div>
    </header>
  );
}
