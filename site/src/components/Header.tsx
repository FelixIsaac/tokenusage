import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { ChevronDown, Github, MoonStar, SunMedium } from "lucide-react";
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
  const [languageMenuOpen, setLanguageMenuOpen] = useState(false);
  const { locale, setLocale, messages } = useI18n();
  const { theme, toggleTheme } = useTheme();
  const themeLabels = THEME_LABELS[locale];
  const languageMenuRef = useRef<HTMLDivElement>(null);
  const currentLanguage = useMemo(
    () => LANGUAGE_OPTIONS.find((option) => option.code === locale) ?? LANGUAGE_OPTIONS[0],
    [locale],
  );

  const scrollTo = useCallback((id: string) => {
    document.getElementById(id)?.scrollIntoView({ behavior: "smooth" });
  }, []);

  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 20);
    window.addEventListener("scroll", onScroll, { passive: true });
    return () => window.removeEventListener("scroll", onScroll);
  }, []);

  useEffect(() => {
    const onPointerDown = (event: MouseEvent) => {
      if (!languageMenuRef.current?.contains(event.target as Node)) {
        setLanguageMenuOpen(false);
      }
    };

    document.addEventListener("mousedown", onPointerDown);
    return () => document.removeEventListener("mousedown", onPointerDown);
  }, []);

  return (
    <header
      className={`theme-header sticky top-3 z-50 mx-auto mt-3 mb-4 flex w-[calc(100vw-28px)] max-w-[min(1280px,calc(100vw-48px))] items-center justify-between gap-3 rounded-full border px-3 py-1.5 backdrop-blur-xl transition-all duration-300 md:top-0 md:mt-0 md:mb-0 md:w-auto md:gap-6 md:px-4 ${
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
          className="h-7 w-7 drop-shadow-[0_0_18px_rgba(99,231,225,0.14)] md:h-8 md:w-8"
        />
      </button>

      <nav className="hidden items-center gap-5 text-text-soft text-[0.95rem] md:flex" aria-label={messages.header.navAria}>
        {messages.header.nav.map((link) => (
          <button key={link.target} onClick={() => scrollTo(link.target)} className="cursor-pointer bg-transparent border-none text-inherit text-[0.95rem] transition-colors hover:text-text-primary">
            {link.label}
          </button>
        ))}
      </nav>

      <div className="ml-auto flex min-w-0 items-center gap-1.5 sm:gap-2.5">
        <div className="relative md:hidden" ref={languageMenuRef}>
          <button
            type="button"
            onClick={() => setLanguageMenuOpen((open) => !open)}
            aria-label={messages.header.languageSwitcherAria}
            aria-haspopup="menu"
            aria-expanded={languageMenuOpen}
            className="theme-toolbar-group inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1.5 font-[family-name:var(--font-display)] text-[0.72rem] tracking-[0.12em] text-text-soft transition-all sm:px-3"
          >
            <span>{currentLanguage.label}</span>
            <ChevronDown
              size={14}
              className={`transition-transform duration-200 ${languageMenuOpen ? "rotate-180" : ""}`}
            />
          </button>

          {languageMenuOpen && (
            <div
              role="menu"
              aria-label={messages.header.languageSwitcherAria}
              className="theme-language-menu absolute right-0 top-[calc(100%+0.55rem)] z-50 grid min-w-[9rem] gap-1 rounded-2xl border p-1.5"
            >
              {LANGUAGE_OPTIONS.map((option) => (
                <button
                  key={option.code}
                  type="button"
                  role="menuitemradio"
                  aria-checked={locale === option.code}
                  onClick={() => {
                    setLocale(option.code);
                    setLanguageMenuOpen(false);
                  }}
                  lang={option.code}
                  title={option.nativeName}
                  data-active={locale === option.code}
                  className="theme-language-menu-item flex items-center justify-between rounded-xl px-3 py-2 font-[family-name:var(--font-display)] text-[0.72rem] tracking-[0.12em] transition-all"
                >
                  <span>{option.label}</span>
                  <span className="theme-language-menu-meta text-[0.7rem] tracking-normal normal-case">
                    {option.nativeName}
                  </span>
                </button>
              ))}
            </div>
          )}
        </div>
        <div
          className="theme-toolbar-group hidden items-center gap-0.5 rounded-full border p-1 md:inline-flex"
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
          className="theme-theme-toggle inline-flex items-center gap-2 rounded-full border px-2.5 py-1.5 font-[family-name:var(--font-display)] text-[0.72rem] tracking-[0.1em] transition-all sm:px-3"
        >
          {theme === "dark" ? <SunMedium size={14} /> : <MoonStar size={14} />}
          <span className="hidden md:inline">{theme === "dark" ? themeLabels.light : themeLabels.dark}</span>
        </button>
        <a
          href="https://github.com/hanbu97/tokenusage"
          target="_blank"
          rel="noreferrer"
          className="theme-button-primary inline-flex shrink-0 items-center gap-2 rounded-full border px-3 py-1.5 font-[family-name:var(--font-display)] text-[0.78rem] tracking-wider transition-all hover:-translate-y-0.5 hover:border-cyan/30 sm:px-3.5"
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
