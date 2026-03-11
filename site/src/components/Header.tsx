import { useState, useEffect, useCallback } from "react";
import { Github, MoonStar, SunMedium } from "lucide-react";
import { LANGUAGE_OPTIONS, useI18n, type Locale } from "../i18n";
import { useTheme } from "../theme";

const THEME_LABELS: Record<Locale, { dark: string; light: string; toggleToDark: string; toggleToLight: string }> = {
  en: { dark: "Dark", light: "Light", toggleToDark: "Switch to dark theme", toggleToLight: "Switch to light theme" },
  fr: { dark: "Sombre", light: "Clair", toggleToDark: "Passer au theme sombre", toggleToLight: "Passer au theme clair" },
  es: { dark: "Oscuro", light: "Claro", toggleToDark: "Cambiar al tema oscuro", toggleToLight: "Cambiar al tema claro" },
  de: { dark: "Dunkel", light: "Hell", toggleToDark: "Zum dunklen Theme wechseln", toggleToLight: "Zum hellen Theme wechseln" },
  zh: { dark: "暗色", light: "亮色", toggleToDark: "切换到暗色主题", toggleToLight: "切换到亮色主题" },
  ja: { dark: "ダーク", light: "ライト", toggleToDark: "ダークテーマに切り替え", toggleToLight: "ライトテーマに切り替え" },
};

export default function Header() {
  const [scrolled, setScrolled] = useState(false);
  const { locale, setLocale, messages } = useI18n();
  const { theme, toggleTheme } = useTheme();
  const themeLabels = THEME_LABELS[locale];

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
      className={`theme-header sticky top-0 z-50 mx-auto flex max-w-[min(1280px,calc(100vw-48px))] items-center justify-between gap-6 rounded-full border px-4 py-1.5 backdrop-blur-xl transition-all duration-300 ${
        scrolled
          ? "theme-header-solid border-line-strong/30 shadow-2xl"
          : "theme-header-resting border-line shadow-lg"
      }`}
    >
      <button
        onClick={() => window.scrollTo({ top: 0, behavior: "smooth" })}
        className="theme-home-button flex items-center rounded-full border p-1 cursor-pointer"
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
          className="theme-toolbar-group inline-flex items-center gap-0.5 rounded-full border p-1"
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
              data-active={locale === option.code}
              className="theme-segment min-w-[2.1rem] rounded-full px-2.5 py-1 font-[family-name:var(--font-display)] text-[0.68rem] tracking-[0.12em] transition-all"
            >
              {option.label}
            </button>
          ))}
        </div>
        <button
          type="button"
          onClick={toggleTheme}
          aria-label={theme === "dark" ? themeLabels.toggleToLight : themeLabels.toggleToDark}
          title={theme === "dark" ? themeLabels.toggleToLight : themeLabels.toggleToDark}
          className="theme-theme-toggle inline-flex items-center gap-2 rounded-full border px-3 py-1.5 font-[family-name:var(--font-display)] text-[0.72rem] tracking-[0.1em] transition-all"
        >
          {theme === "dark" ? <SunMedium size={14} /> : <MoonStar size={14} />}
          <span className="hidden md:inline">{theme === "dark" ? themeLabels.light : themeLabels.dark}</span>
        </button>
        <a
          href="https://github.com/hanbu97/tokenusage"
          target="_blank"
          rel="noreferrer"
          className="theme-button-primary inline-flex items-center gap-2 rounded-full border px-3.5 py-1.5 font-[family-name:var(--font-display)] text-[0.78rem] tracking-wider transition-all hover:-translate-y-0.5 hover:border-cyan/30"
        >
          <Github size={15} />
          <span className="hidden sm:inline">{messages.header.github}</span>
        </a>
        <button
          onClick={() => scrollTo("install")}
          className="theme-button-secondary hidden rounded-full border px-3.5 py-1.5 font-[family-name:var(--font-display)] text-[0.78rem] tracking-wider cursor-pointer transition-all hover:-translate-y-0.5 hover:border-cyan/30 sm:inline-flex"
        >
          {messages.header.install}
        </button>
      </div>
    </header>
  );
}
