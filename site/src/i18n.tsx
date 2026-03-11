import { createContext, useContext, useEffect, useState, type ReactNode } from "react";

export type Locale = "en" | "fr" | "es" | "de" | "zh" | "ja";

export const LANGUAGE_OPTIONS: { code: Locale; label: string; nativeName: string }[] = [
  { code: "en", label: "EN", nativeName: "English" },
  { code: "fr", label: "FR", nativeName: "Francais" },
  { code: "es", label: "ES", nativeName: "Espanol" },
  { code: "de", label: "DE", nativeName: "Deutsch" },
  { code: "zh", label: "ZH", nativeName: "中文" },
  { code: "ja", label: "JA", nativeName: "日本語" },
];

const LOCALE_STORAGE_KEY = "tokenusage.locale";

const EN_MESSAGES = {
  meta: {
    title: "tokenusage - Fast token tracking for Codex, Claude, and AI coding workflows",
    description:
      "tokenusage is a fast local token tracker for Codex, Claude, and Antigravity. See daily cost reports, live rate limits, activity views, heartbeat-powered coding time, a GUI dashboard, and shareable image cards.",
    socialDescription: "Fast local token tracking for Codex, Claude, and AI coding workflows.",
  },
  common: {
    copy: "Copy command",
    copied: "Copied",
    play: "Play",
    replay: "Replay",
    fullscreen: "Fullscreen",
    close: "Close",
  },
  header: {
    homeAria: "tokenusage home",
    navAria: "Primary navigation",
    languageSwitcherAria: "Language selector",
    nav: [
      { label: "Tour", target: "tour" },
      { label: "Install", target: "install" },
      { label: "Docs", target: "docs" },
    ],
    github: "GitHub",
    install: "Install",
  },
  hero: {
    title: "TOKENUSAGE",
    subtitleLead: "Tokens, limits, and activity in one",
    subtitleAccent: "fast local workflow.",
    worksWith: "Works with",
    supported: "Supported",
    explore: "Explore",
    starOnGithub: "Star on GitHub",
    installBadge: "Install",
    installLead: "One line, then run",
    tags: {
      faster: {
        label: "214x Faster",
        title: "Rust-native speed",
        details: [
          "214x faster than ccusage on real-world logs",
          "Zero-copy JSONL parsing pipeline",
          "Sub-second results on 100k+ log entries",
        ],
      },
      local: {
        label: "100% Local",
        title: "No network calls, ever",
        details: [
          "All parsing runs on your machine",
          "Logs never leave your disk",
          "Works offline, no API keys needed",
        ],
      },
      sources: {
        label: "3 Sources",
        title: "Unified reporting",
        details: [
          "Claude Code + OpenAI Codex + Antigravity",
          "Single merged token report",
          "Cross-provider daily and weekly summaries",
        ],
      },
      live: {
        label: "Live Monitor",
        title: "Real-time tracking",
        details: [
          "tu live streams token usage as you code",
          "Rate limit warnings before you hit walls",
          "Heartbeat-backed activity time tracking",
        ],
      },
      share: {
        label: "Share Cards",
        title: "Show off your usage",
        details: [
          "Generate beautiful PNG share cards",
          "tu img day / tu img week",
          "Perfect for social media and team updates",
        ],
      },
    },
    visualAlts: {
      gui: "tokenusage GUI dashboard",
      cli: "tokenusage CLI report",
      shareWeek: "tokenusage weekly share card",
      shareDay: "tokenusage share card",
      live: "tokenusage live monitor",
    },
  },
  metrics: [
    { value: "0.08s", label: "Warm run on 1,521-file, 2.2 GB Claude log set" },
    { value: "214x", label: "0.08s vs 17.15s, Rust + parallel scan + caching" },
    { value: "3 sources", label: "Codex, Claude, and Antigravity in one merged view" },
    { value: "100% local", label: "Logs stay on your machine. Only pricing metadata is fetched." },
  ],
  tour: {
    badge: "Tour",
    title: "Every command, one workflow.",
    description: "Scroll through each feature to see how tokenusage covers your entire AI coding workflow.",
    panels: {
      daily: {
        panelTitle: "Merged CLI report",
        stepTitle: "Merged token report",
        linkLabel: "Quick start",
      },
      live: {
        panelTitle: "Live monitor",
        stepTitle: "Real-time TUI monitor",
        linkLabel: "Live docs",
      },
      top: {
        panelTitle: "Session top",
        stepTitle: "htop for tokens",
        linkLabel: "Top docs",
      },
      today: {
        panelTitle: "Today view",
        stepTitle: "Today's coding activity",
        linkLabel: "Activity docs",
      },
      activity: {
        panelTitle: "Activity view",
        stepTitle: "Multi-day activity breakdown",
        linkLabel: "Activity docs",
      },
      heartbeat: {
        panelTitle: "Heartbeat system",
        stepTitle: "Local heartbeat collector",
        linkLabel: "Heartbeat docs",
      },
      img: {
        panelTitle: "Share cards",
        stepTitle: "Shareable image cards",
        linkLabel: "Image docs",
      },
      gui: {
        panelTitle: "GUI dashboard",
        stepTitle: "Desktop GUI dashboard",
        linkLabel: "GUI docs",
      },
      periods: {
        panelTitle: "Period reports",
        stepTitle: "Weekly and monthly reports",
        linkLabel: "Period docs",
      },
      statusline: {
        panelTitle: "Statusline",
        stepTitle: "Editor and tmux integration",
        linkLabel: "Statusline docs",
        contentHeading: "Embed in your workflow",
        contentBody:
          "Outputs a compact status string for tmux, Neovim, or any tool that reads shell output. Includes session cost, limits, and burn rate.",
      },
    },
    imageAlts: {
      dailyShare: "Daily share card",
      weeklyShare: "Weekly share card",
      gui: "GUI dashboard demo",
    },
  },
  install: {
    badge: "Install",
    title: "One install. CLI, TUI, or GUI.",
    thenRun: "Then run",
    commands: [
      { cmd: "tu", tip: "Merged token report" },
      { cmd: "tu live codex", tip: "Real-time limits and pace" },
      { cmd: "tu today", tip: "Activity view for today" },
      { cmd: "tu img week", tip: "Generate weekly share card" },
      { cmd: "tu gui", tip: "Open GUI dashboard" },
      { cmd: "tu heartbeat watch .", tip: "Track coding activity" },
    ],
  },
  docs: {
    badge: "Docs",
    title: "Docs, source, and registries.",
    description: "Everything you need to get started, compare options, and install from your preferred registry.",
    links: [
      {
        label: "Quick start",
        title: "Command-first onboarding",
        desc: "Install, run, go live, turn on activity, and generate share cards.",
      },
      {
        label: "Comparison",
        title: "tokenusage vs ccusage",
        desc: "Performance, merged-source reporting, live monitoring, and product surface area.",
      },
      {
        label: "Registry",
        title: "Rust crate",
        desc: "Install from crates.io or cargo-binstall and use tu locally.",
      },
      {
        label: "Registry",
        title: "npm package",
        desc: "Global install path for users who want one-line onboarding.",
      },
    ],
  },
  footer: {
    badge: "Ready",
    title: "Track your tokens. Ship more code.",
    description: "One install, every token metric you need. Fast, local, and open source.",
    starOnGithub: "Star on GitHub",
    contact: "Contact us",
    builtWith: "tokenusage · Built with Rust",
  },
};

type Messages = typeof EN_MESSAGES;

const TRANSLATIONS: Record<Locale, Messages> = {
  en: EN_MESSAGES,
  fr: {
    meta: {
      title: "tokenusage - Suivi ultra-rapide des tokens pour Codex, Claude et les workflows IA",
      description:
        "tokenusage est un traceur local rapide pour Codex, Claude et Antigravity. Consultez les rapports de cout quotidiens, les limites en direct, les vues d'activite, le temps de code base sur les heartbeats, un tableau de bord GUI et des cartes image partageables.",
      socialDescription: "Suivi local et rapide des tokens pour Codex, Claude et les workflows de code IA.",
    },
    common: {
      copy: "Copier la commande",
      copied: "Copie",
      play: "Lire",
      replay: "Relancer",
      fullscreen: "Plein ecran",
      close: "Fermer",
    },
    header: {
      homeAria: "accueil tokenusage",
      navAria: "Navigation principale",
      languageSwitcherAria: "Selecteur de langue",
      nav: [
        { label: "Visite", target: "tour" },
        { label: "Installation", target: "install" },
        { label: "Docs", target: "docs" },
      ],
      github: "GitHub",
      install: "Installer",
    },
    hero: {
      title: "TOKENUSAGE",
      subtitleLead: "Tokens, limites et activite dans un",
      subtitleAccent: "workflow local ultra-rapide.",
      worksWith: "Compatible avec",
      supported: "Pris en charge",
      explore: "Explorer",
      starOnGithub: "Star sur GitHub",
      installBadge: "Installation",
      installLead: "Une ligne, puis lancez",
      tags: {
        faster: {
          label: "214x Plus Rapide",
          title: "Vitesse native Rust",
          details: [
            "214x plus rapide que ccusage sur des logs reels",
            "Pipeline JSONL zero-copy",
            "Resultats en moins d'une seconde sur plus de 100k lignes",
          ],
        },
        local: {
          label: "100% Local",
          title: "Aucun appel reseau",
          details: [
            "Toute l'analyse tourne sur votre machine",
            "Les logs ne quittent jamais votre disque",
            "Fonctionne hors ligne, sans cle API",
          ],
        },
        sources: {
          label: "3 Sources",
          title: "Reporting unifie",
          details: [
            "Claude Code + OpenAI Codex + Antigravity",
            "Un seul rapport fusionne",
            "Resumes quotidiens et hebdomadaires multi-fournisseurs",
          ],
        },
        live: {
          label: "Moniteur Live",
          title: "Suivi en temps reel",
          details: [
            "tu live diffuse l'usage pendant que vous codez",
            "Alertes avant d'atteindre les limites",
            "Suivi du temps base sur les heartbeats",
          ],
        },
        share: {
          label: "Cartes Partage",
          title: "Montrez vos chiffres",
          details: [
            "Generez de belles cartes PNG",
            "tu img day / tu img week",
            "Parfait pour les reseaux sociaux et l'equipe",
          ],
        },
      },
      visualAlts: {
        gui: "tableau de bord GUI tokenusage",
        cli: "rapport CLI tokenusage",
        shareWeek: "carte hebdomadaire tokenusage",
        shareDay: "carte de partage tokenusage",
        live: "moniteur live tokenusage",
      },
    },
    metrics: [
      { value: "0.08s", label: "Execution a chaud sur 1 521 fichiers et 2,2 Go de logs Claude" },
      { value: "214x", label: "0,08s contre 17,15s, Rust + scan parallele + cache" },
      { value: "3 sources", label: "Codex, Claude et Antigravity dans une vue fusionnee" },
      { value: "100% local", label: "Les logs restent sur votre machine. Seules les metadonnees de prix sont telechargees." },
    ],
    tour: {
      badge: "Visite",
      title: "Chaque commande, un seul workflow.",
      description: "Parcourez chaque fonctionnalite pour voir comment tokenusage couvre tout votre flux de code assiste par IA.",
      panels: {
        daily: { panelTitle: "Rapport CLI fusionne", stepTitle: "Rapport de tokens fusionne", linkLabel: "Demarrage rapide" },
        live: { panelTitle: "Moniteur live", stepTitle: "Moniteur TUI en temps reel", linkLabel: "Docs live" },
        top: { panelTitle: "Top de session", stepTitle: "htop pour les tokens", linkLabel: "Docs top" },
        today: { panelTitle: "Vue du jour", stepTitle: "Activite de code du jour", linkLabel: "Docs activite" },
        activity: { panelTitle: "Vue activite", stepTitle: "Analyse d'activite sur plusieurs jours", linkLabel: "Docs activite" },
        heartbeat: { panelTitle: "Systeme heartbeat", stepTitle: "Collecteur heartbeat local", linkLabel: "Docs heartbeat" },
        img: { panelTitle: "Cartes partage", stepTitle: "Cartes image partageables", linkLabel: "Docs image" },
        gui: { panelTitle: "Tableau de bord GUI", stepTitle: "Tableau de bord desktop", linkLabel: "Docs GUI" },
        periods: { panelTitle: "Rapports de periode", stepTitle: "Rapports hebdo et mensuels", linkLabel: "Docs periode" },
        statusline: {
          panelTitle: "Statusline",
          stepTitle: "Integration editeur et tmux",
          linkLabel: "Docs statusline",
          contentHeading: "Integrez-le a votre workflow",
          contentBody:
            "Produit une chaine compacte pour tmux, Neovim ou tout outil lisant la sortie shell. Inclut cout de session, limites et rythme de consommation.",
        },
      },
      imageAlts: {
        dailyShare: "carte de partage quotidienne",
        weeklyShare: "carte de partage hebdomadaire",
        gui: "demo du tableau de bord GUI",
      },
    },
    install: {
      badge: "Installation",
      title: "Une installation. CLI, TUI ou GUI.",
      thenRun: "Puis lancez",
      commands: [
        { cmd: "tu", tip: "Rapport de tokens fusionne" },
        { cmd: "tu live codex", tip: "Limites et rythme en temps reel" },
        { cmd: "tu today", tip: "Vue activite du jour" },
        { cmd: "tu img week", tip: "Generer la carte hebdomadaire" },
        { cmd: "tu gui", tip: "Ouvrir le tableau de bord GUI" },
        { cmd: "tu heartbeat watch .", tip: "Suivre l'activite de code" },
      ],
    },
    docs: {
      badge: "Docs",
      title: "Docs, source et registres.",
      description: "Tout ce qu'il faut pour demarrer, comparer les options et installer depuis votre registre prefere.",
      links: [
        {
          label: "Demarrage rapide",
          title: "Onboarding centre commande",
          desc: "Installez, lancez, passez en live, activez l'activite et generez des cartes partage.",
        },
        {
          label: "Comparaison",
          title: "tokenusage vs ccusage",
          desc: "Performance, rapports fusionnes, monitoring live et surface produit.",
        },
        {
          label: "Registre",
          title: "Crate Rust",
          desc: "Installez depuis crates.io ou cargo-binstall et utilisez tu en local.",
        },
        {
          label: "Registre",
          title: "Paquet npm",
          desc: "Voie d'installation globale pour un demarrage en une ligne.",
        },
      ],
    },
    footer: {
      badge: "Pret",
      title: "Suivez vos tokens. Livrez plus de code.",
      description: "Une installation, toutes les mesures de tokens dont vous avez besoin. Rapide, local et open source.",
      starOnGithub: "Star sur GitHub",
      contact: "Nous contacter",
      builtWith: "tokenusage · Construit avec Rust",
    },
  },
  es: {
    meta: {
      title: "tokenusage - Seguimiento rapido de tokens para Codex, Claude y flujos de trabajo con IA",
      description:
        "tokenusage es un rastreador local y rapido de tokens para Codex, Claude y Antigravity. Consulta reportes diarios de costo, limites en vivo, vistas de actividad, tiempo de programacion con heartbeat, un panel GUI y tarjetas compartibles.",
      socialDescription: "Seguimiento local y rapido de tokens para Codex, Claude y flujos de programacion con IA.",
    },
    common: {
      copy: "Copiar comando",
      copied: "Copiado",
      play: "Reproducir",
      replay: "Repetir",
      fullscreen: "Pantalla completa",
      close: "Cerrar",
    },
    header: {
      homeAria: "inicio de tokenusage",
      navAria: "Navegacion principal",
      languageSwitcherAria: "Selector de idioma",
      nav: [
        { label: "Tour", target: "tour" },
        { label: "Instalar", target: "install" },
        { label: "Docs", target: "docs" },
      ],
      github: "GitHub",
      install: "Instalar",
    },
    hero: {
      title: "TOKENUSAGE",
      subtitleLead: "Tokens, limites y actividad en un",
      subtitleAccent: "flujo local y rapido.",
      worksWith: "Funciona con",
      supported: "Compatible",
      explore: "Explorar",
      starOnGithub: "Star en GitHub",
      installBadge: "Instalar",
      installLead: "Una linea y luego ejecuta",
      tags: {
        faster: {
          label: "214x Mas Rapido",
          title: "Velocidad nativa en Rust",
          details: [
            "214x mas rapido que ccusage en logs reales",
            "Pipeline JSONL zero-copy",
            "Resultados en menos de un segundo con mas de 100k entradas",
          ],
        },
        local: {
          label: "100% Local",
          title: "Sin llamadas de red",
          details: [
            "Todo el parseo corre en tu maquina",
            "Los logs nunca salen de tu disco",
            "Funciona offline, sin API keys",
          ],
        },
        sources: {
          label: "3 Fuentes",
          title: "Reportes unificados",
          details: [
            "Claude Code + OpenAI Codex + Antigravity",
            "Un solo reporte combinado",
            "Resumenes diarios y semanales entre proveedores",
          ],
        },
        live: {
          label: "Monitor Live",
          title: "Seguimiento en tiempo real",
          details: [
            "tu live transmite el uso mientras programas",
            "Alertas antes de tocar el limite",
            "Tiempo de actividad basado en heartbeats",
          ],
        },
        share: {
          label: "Tarjetas Share",
          title: "Muestra tu uso",
          details: [
            "Genera tarjetas PNG atractivas",
            "tu img day / tu img week",
            "Perfecto para redes sociales y actualizaciones de equipo",
          ],
        },
      },
      visualAlts: {
        gui: "panel GUI de tokenusage",
        cli: "reporte CLI de tokenusage",
        shareWeek: "tarjeta semanal de tokenusage",
        shareDay: "tarjeta compartible de tokenusage",
        live: "monitor en vivo de tokenusage",
      },
    },
    metrics: [
      { value: "0.08s", label: "Ejecucion en caliente sobre 1.521 archivos y 2,2 GB de logs de Claude" },
      { value: "214x", label: "0,08s frente a 17,15s, Rust + escaneo paralelo + cache" },
      { value: "3 sources", label: "Codex, Claude y Antigravity en una sola vista combinada" },
      { value: "100% local", label: "Los logs se quedan en tu maquina. Solo se obtienen metadatos de precios." },
    ],
    tour: {
      badge: "Tour",
      title: "Cada comando, un solo flujo.",
      description: "Recorre cada funcion para ver como tokenusage cubre todo tu flujo de programacion con IA.",
      panels: {
        daily: { panelTitle: "Reporte CLI combinado", stepTitle: "Reporte combinado de tokens", linkLabel: "Inicio rapido" },
        live: { panelTitle: "Monitor live", stepTitle: "Monitor TUI en tiempo real", linkLabel: "Docs live" },
        top: { panelTitle: "Top de sesion", stepTitle: "htop para tokens", linkLabel: "Docs top" },
        today: { panelTitle: "Vista de hoy", stepTitle: "Actividad de codigo de hoy", linkLabel: "Docs actividad" },
        activity: { panelTitle: "Vista de actividad", stepTitle: "Desglose de actividad de varios dias", linkLabel: "Docs actividad" },
        heartbeat: { panelTitle: "Sistema heartbeat", stepTitle: "Colector heartbeat local", linkLabel: "Docs heartbeat" },
        img: { panelTitle: "Tarjetas share", stepTitle: "Tarjetas de imagen compartibles", linkLabel: "Docs imagen" },
        gui: { panelTitle: "Panel GUI", stepTitle: "Panel GUI de escritorio", linkLabel: "Docs GUI" },
        periods: { panelTitle: "Reportes por periodo", stepTitle: "Reportes semanales y mensuales", linkLabel: "Docs periodo" },
        statusline: {
          panelTitle: "Statusline",
          stepTitle: "Integracion con editor y tmux",
          linkLabel: "Docs statusline",
          contentHeading: "Integralo a tu flujo",
          contentBody:
            "Genera una cadena compacta para tmux, Neovim o cualquier herramienta que lea salida del shell. Incluye costo de sesion, limites y ritmo de consumo.",
        },
      },
      imageAlts: {
        dailyShare: "tarjeta diaria para compartir",
        weeklyShare: "tarjeta semanal para compartir",
        gui: "demo del panel GUI",
      },
    },
    install: {
      badge: "Instalar",
      title: "Una instalacion. CLI, TUI o GUI.",
      thenRun: "Luego ejecuta",
      commands: [
        { cmd: "tu", tip: "Reporte combinado de tokens" },
        { cmd: "tu live codex", tip: "Limites y ritmo en tiempo real" },
        { cmd: "tu today", tip: "Vista de actividad de hoy" },
        { cmd: "tu img week", tip: "Generar tarjeta semanal" },
        { cmd: "tu gui", tip: "Abrir panel GUI" },
        { cmd: "tu heartbeat watch .", tip: "Seguir actividad de codigo" },
      ],
    },
    docs: {
      badge: "Docs",
      title: "Docs, codigo fuente y registros.",
      description: "Todo lo que necesitas para empezar, comparar opciones e instalar desde tu registro preferido.",
      links: [
        {
          label: "Inicio rapido",
          title: "Onboarding centrado en comandos",
          desc: "Instala, ejecuta, entra en live, activa actividad y genera tarjetas compartibles.",
        },
        {
          label: "Comparacion",
          title: "tokenusage vs ccusage",
          desc: "Rendimiento, reportes combinados, monitoreo en vivo y superficie de producto.",
        },
        {
          label: "Registro",
          title: "Crate de Rust",
          desc: "Instala desde crates.io o cargo-binstall y usa tu en local.",
        },
        {
          label: "Registro",
          title: "Paquete npm",
          desc: "Ruta de instalacion global para quien quiere empezar en una sola linea.",
        },
      ],
    },
    footer: {
      badge: "Listo",
      title: "Sigue tus tokens. Entrega mas codigo.",
      description: "Una instalacion, todas las metricas de tokens que necesitas. Rapido, local y open source.",
      starOnGithub: "Star en GitHub",
      contact: "Contactanos",
      builtWith: "tokenusage · Hecho con Rust",
    },
  },
  de: {
    meta: {
      title: "tokenusage - Schnelles Token-Tracking fur Codex, Claude und KI-Workflows",
      description:
        "tokenusage ist ein schneller lokaler Token-Tracker fur Codex, Claude und Antigravity. Sieh dir tagliche Kostenberichte, Live-Limits, Aktivitatsansichten, heartbeat-basiertes Coding, ein GUI-Dashboard und teilbare Bildkarten an.",
      socialDescription: "Schnelles lokales Token-Tracking fur Codex, Claude und KI-Coding-Workflows.",
    },
    common: {
      copy: "Befehl kopieren",
      copied: "Kopiert",
      play: "Abspielen",
      replay: "Neu starten",
      fullscreen: "Vollbild",
      close: "Schliessen",
    },
    header: {
      homeAria: "tokenusage Startseite",
      navAria: "Hauptnavigation",
      languageSwitcherAria: "Sprachauswahl",
      nav: [
        { label: "Tour", target: "tour" },
        { label: "Installieren", target: "install" },
        { label: "Docs", target: "docs" },
      ],
      github: "GitHub",
      install: "Installieren",
    },
    hero: {
      title: "TOKENUSAGE",
      subtitleLead: "Tokens, Limits und Aktivitat in einem",
      subtitleAccent: "schnellen lokalen Workflow.",
      worksWith: "Funktioniert mit",
      supported: "Unterstutzt",
      explore: "Entdecken",
      starOnGithub: "Star auf GitHub",
      installBadge: "Installation",
      installLead: "Eine Zeile, dann starte",
      tags: {
        faster: {
          label: "214x Schneller",
          title: "Rust-native Geschwindigkeit",
          details: [
            "214x schneller als ccusage bei echten Logs",
            "Zero-copy JSONL-Pipeline",
            "Ergebnisse in unter einer Sekunde bei uber 100k Eintragen",
          ],
        },
        local: {
          label: "100% Lokal",
          title: "Keine Netzwerkaufrufe",
          details: [
            "Das komplette Parsing lauft auf deinem Rechner",
            "Logs verlassen niemals deine Festplatte",
            "Funktioniert offline, ohne API-Schlussel",
          ],
        },
        sources: {
          label: "3 Quellen",
          title: "Vereintes Reporting",
          details: [
            "Claude Code + OpenAI Codex + Antigravity",
            "Ein zusammengefuhrter Token-Report",
            "Tages- und Wochenberichte uber mehrere Anbieter",
          ],
        },
        live: {
          label: "Live Monitor",
          title: "Tracking in Echtzeit",
          details: [
            "tu live streamt den Verbrauch wahrend du codest",
            "Warnungen vor dem Limit",
            "Aktivitatstracking auf Heartbeat-Basis",
          ],
        },
        share: {
          label: "Share Cards",
          title: "Zeig deinen Verbrauch",
          details: [
            "Erzeuge schone PNG-Karten",
            "tu img day / tu img week",
            "Perfekt fur Social Media und Team-Updates",
          ],
        },
      },
      visualAlts: {
        gui: "tokenusage GUI-Dashboard",
        cli: "tokenusage CLI-Report",
        shareWeek: "wochentliche tokenusage Share-Karte",
        shareDay: "tokenusage Share-Karte",
        live: "tokenusage Live-Monitor",
      },
    },
    metrics: [
      { value: "0.08s", label: "Warmer Lauf auf einem Claude-Logsatz mit 1.521 Dateien und 2,2 GB" },
      { value: "214x", label: "0,08s statt 17,15s, Rust + paralleler Scan + Caching" },
      { value: "3 Quellen", label: "Codex, Claude und Antigravity in einer zusammengefuhrten Ansicht" },
      { value: "100% lokal", label: "Logs bleiben auf deinem Rechner. Nur Preismetadaten werden geladen." },
    ],
    tour: {
      badge: "Tour",
      title: "Jeder Befehl, ein Workflow.",
      description: "Scrolle durch jede Funktion und sieh, wie tokenusage deinen gesamten KI-Coding-Workflow abdeckt.",
      panels: {
        daily: { panelTitle: "Zusammengefuhrter CLI-Report", stepTitle: "Zusammengefuhrter Token-Report", linkLabel: "Schnellstart" },
        live: { panelTitle: "Live-Monitor", stepTitle: "TUI-Monitor in Echtzeit", linkLabel: "Live-Doku" },
        top: { panelTitle: "Session-Top", stepTitle: "htop fur Tokens", linkLabel: "Top-Doku" },
        today: { panelTitle: "Heute-Ansicht", stepTitle: "Heutige Coding-Aktivitat", linkLabel: "Aktivitats-Doku" },
        activity: { panelTitle: "Aktivitatsansicht", stepTitle: "Mehrtagige Aktivitatsanalyse", linkLabel: "Aktivitats-Doku" },
        heartbeat: { panelTitle: "Heartbeat-System", stepTitle: "Lokaler Heartbeat-Sammler", linkLabel: "Heartbeat-Doku" },
        img: { panelTitle: "Share Cards", stepTitle: "Teilbare Bildkarten", linkLabel: "Bild-Doku" },
        gui: { panelTitle: "GUI-Dashboard", stepTitle: "Desktop-GUI-Dashboard", linkLabel: "GUI-Doku" },
        periods: { panelTitle: "Zeitraum-Berichte", stepTitle: "Wochen- und Monatsberichte", linkLabel: "Zeitraum-Doku" },
        statusline: {
          panelTitle: "Statusline",
          stepTitle: "Editor- und tmux-Integration",
          linkLabel: "Statusline-Doku",
          contentHeading: "In deinen Workflow einbetten",
          contentBody:
            "Erzeugt eine kompakte Statuszeile fur tmux, Neovim oder jedes Tool, das Shell-Ausgaben liest. Enthalt Sitzungskosten, Limits und Verbrauchstempo.",
        },
      },
      imageAlts: {
        dailyShare: "tagliche Share-Karte",
        weeklyShare: "wochentliche Share-Karte",
        gui: "GUI-Dashboard-Demo",
      },
    },
    install: {
      badge: "Installation",
      title: "Eine Installation. CLI, TUI oder GUI.",
      thenRun: "Dann starten",
      commands: [
        { cmd: "tu", tip: "Zusammengefuhrter Token-Report" },
        { cmd: "tu live codex", tip: "Limits und Tempo in Echtzeit" },
        { cmd: "tu today", tip: "Aktivitatsansicht fur heute" },
        { cmd: "tu img week", tip: "Wochentliche Share-Karte erzeugen" },
        { cmd: "tu gui", tip: "GUI-Dashboard offnen" },
        { cmd: "tu heartbeat watch .", tip: "Coding-Aktivitat verfolgen" },
      ],
    },
    docs: {
      badge: "Docs",
      title: "Docs, Source und Registries.",
      description: "Alles, was du fur den Einstieg, den Vergleich und die Installation aus deiner bevorzugten Registry brauchst.",
      links: [
        {
          label: "Schnellstart",
          title: "Onboarding per Kommando",
          desc: "Installieren, starten, live gehen, Aktivitat aktivieren und Share Cards erzeugen.",
        },
        {
          label: "Vergleich",
          title: "tokenusage vs ccusage",
          desc: "Performance, zusammengefuhrtes Reporting, Live-Monitoring und Produktumfang.",
        },
        {
          label: "Registry",
          title: "Rust-Crate",
          desc: "Installiere uber crates.io oder cargo-binstall und nutze tu lokal.",
        },
        {
          label: "Registry",
          title: "npm-Paket",
          desc: "Globaler Installationspfad fur Nutzer, die in einer Zeile starten wollen.",
        },
      ],
    },
    footer: {
      badge: "Bereit",
      title: "Verfolge deine Tokens. Liefere mehr Code.",
      description: "Eine Installation, alle Token-Metriken, die du brauchst. Schnell, lokal und Open Source.",
      starOnGithub: "Star auf GitHub",
      contact: "Kontakt",
      builtWith: "tokenusage · Mit Rust gebaut",
    },
  },
  zh: {
    meta: {
      title: "tokenusage - 面向 Codex、Claude 与 AI 编码工作流的高速 Token 跟踪工具",
      description:
        "tokenusage 是面向 Codex、Claude 和 Antigravity 的本地高速 token 跟踪工具。你可以查看每日成本报告、实时限额、活动视图、基于 heartbeat 的编码时长、GUI 仪表盘，以及可分享的图片卡片。",
      socialDescription: "面向 Codex、Claude 与 AI 编码工作流的本地高速 Token 跟踪工具。",
    },
    common: {
      copy: "复制命令",
      copied: "已复制",
      play: "播放",
      replay: "重播",
      fullscreen: "全屏",
      close: "关闭",
    },
    header: {
      homeAria: "tokenusage 首页",
      navAria: "主导航",
      languageSwitcherAria: "语言切换",
      nav: [
        { label: "导览", target: "tour" },
        { label: "安装", target: "install" },
        { label: "文档", target: "docs" },
      ],
      github: "GitHub",
      install: "安装",
    },
    hero: {
      title: "TOKENUSAGE",
      subtitleLead: "把 Tokens、限额与活动数据放进同一个",
      subtitleAccent: "高性能本地工作流。",
      worksWith: "适配平台",
      supported: "已支持",
      explore: "继续查看",
      starOnGithub: "GitHub Star",
      installBadge: "安装",
      installLead: "一行安装，然后运行",
      tags: {
        faster: {
          label: "214x 更快",
          title: "Rust 原生速度",
          details: [
            "真实日志下比 ccusage 快 214 倍",
            "Zero-copy JSONL 解析管线",
            "10 万条以上日志也能亚秒级返回",
          ],
        },
        local: {
          label: "100% 本地",
          title: "完全无需联网",
          details: [
            "所有解析都在本机完成",
            "日志不会离开你的磁盘",
            "离线可用，不需要 API Key",
          ],
        },
        sources: {
          label: "3 个来源",
          title: "统一报告",
          details: [
            "Claude Code + OpenAI Codex + Antigravity",
            "单一合并 Token 报告",
            "跨提供商的日周汇总",
          ],
        },
        live: {
          label: "实时监控",
          title: "实时跟踪",
          details: [
            "tu live 在编码时持续输出用量",
            "在触顶前提前提示限额风险",
            "基于 heartbeat 的活动时长追踪",
          ],
        },
        share: {
          label: "分享卡片",
          title: "把你的用量展示出来",
          details: [
            "生成精美的 PNG 分享卡片",
            "tu img day / tu img week",
            "适合社交媒体和团队周报",
          ],
        },
      },
      visualAlts: {
        gui: "tokenusage GUI 仪表盘",
        cli: "tokenusage CLI 报告",
        shareWeek: "tokenusage 周分享卡片",
        shareDay: "tokenusage 分享卡片",
        live: "tokenusage 实时监控",
      },
    },
    metrics: [
      { value: "0.08s", label: "在包含 1,521 个文件、2.2 GB 的 Claude 日志集上热启动仅需 0.08 秒" },
      { value: "214x", label: "0.08 秒对比 17.15 秒，来自 Rust + 并行扫描 + 缓存" },
      { value: "3 个来源", label: "Codex、Claude 和 Antigravity 汇总到一个视图" },
      { value: "100% 本地", label: "日志保留在你的机器上。仅会拉取价格元数据。" },
    ],
    tour: {
      badge: "导览",
      title: "每条命令，都在同一个工作流里。",
      description: "往下滚动查看每个功能，了解 tokenusage 如何覆盖完整的 AI 编码工作流。",
      panels: {
        daily: { panelTitle: "合并 CLI 报告", stepTitle: "合并 Token 报告", linkLabel: "快速开始" },
        live: { panelTitle: "实时监控", stepTitle: "实时 TUI 监视器", linkLabel: "实时文档" },
        top: { panelTitle: "会话排行", stepTitle: "Token 版 htop", linkLabel: "排行文档" },
        today: { panelTitle: "今日视图", stepTitle: "今天的编码活动", linkLabel: "活动文档" },
        activity: { panelTitle: "活动视图", stepTitle: "多日活动拆解", linkLabel: "活动文档" },
        heartbeat: { panelTitle: "Heartbeat 系统", stepTitle: "本地 heartbeat 采集器", linkLabel: "Heartbeat 文档" },
        img: { panelTitle: "分享卡片", stepTitle: "可分享图片卡片", linkLabel: "图片文档" },
        gui: { panelTitle: "GUI 仪表盘", stepTitle: "桌面 GUI 仪表盘", linkLabel: "GUI 文档" },
        periods: { panelTitle: "周期报告", stepTitle: "周报与月报", linkLabel: "周期文档" },
        statusline: {
          panelTitle: "状态栏",
          stepTitle: "编辑器与 tmux 集成",
          linkLabel: "状态栏文档",
          contentHeading: "嵌入你的工作流",
          contentBody:
            "输出紧凑状态字符串，可直接用于 tmux、Neovim 或任何读取 shell 输出的工具。包含会话成本、限额与消耗速度。",
        },
      },
      imageAlts: {
        dailyShare: "日分享卡片",
        weeklyShare: "周分享卡片",
        gui: "GUI 仪表盘演示",
      },
    },
    install: {
      badge: "安装",
      title: "一次安装。CLI、TUI、GUI 全都能用。",
      thenRun: "然后运行",
      commands: [
        { cmd: "tu", tip: "合并 Token 报告" },
        { cmd: "tu live codex", tip: "实时查看限额与消耗节奏" },
        { cmd: "tu today", tip: "查看今天的活动视图" },
        { cmd: "tu img week", tip: "生成周分享卡片" },
        { cmd: "tu gui", tip: "打开 GUI 仪表盘" },
        { cmd: "tu heartbeat watch .", tip: "追踪编码活动" },
      ],
    },
    docs: {
      badge: "文档",
      title: "文档、源码与注册表。",
      description: "开始使用、横向对比以及从你偏好的注册表安装，所需信息都在这里。",
      links: [
        {
          label: "快速开始",
          title: "命令优先上手",
          desc: "安装、运行、进入实时模式、开启活动跟踪，并生成分享卡片。",
        },
        {
          label: "对比",
          title: "tokenusage vs ccusage",
          desc: "性能、聚合报告、实时监控，以及产品能力面的全面对比。",
        },
        {
          label: "注册表",
          title: "Rust crate",
          desc: "通过 crates.io 或 cargo-binstall 安装，并在本地直接使用 tu。",
        },
        {
          label: "注册表",
          title: "npm 包",
          desc: "适合希望一行命令快速完成安装的用户。",
        },
      ],
    },
    footer: {
      badge: "准备好了",
      title: "把 Token 管起来，把代码交付得更快。",
      description: "一次安装，拿到你需要的全部 Token 指标。快速、本地、开源。",
      starOnGithub: "GitHub Star",
      contact: "联系我们",
      builtWith: "tokenusage · 基于 Rust 构建",
    },
  },
  ja: {
    meta: {
      title: "tokenusage - Codex、Claude、AI コーディング向けの高速トークン追跡",
      description:
        "tokenusage は Codex、Claude、Antigravity 向けの高速ローカルトークントラッカーです。日次コストレポート、ライブ制限、アクティビティ表示、heartbeat ベースの作業時間、GUI ダッシュボード、共有用イメージカードを確認できます。",
      socialDescription: "Codex、Claude、AI コーディング向けの高速ローカルトークン追跡。",
    },
    common: {
      copy: "コマンドをコピー",
      copied: "コピー済み",
      play: "再生",
      replay: "もう一度",
      fullscreen: "全画面",
      close: "閉じる",
    },
    header: {
      homeAria: "tokenusage ホーム",
      navAria: "メインナビゲーション",
      languageSwitcherAria: "言語切り替え",
      nav: [
        { label: "ツアー", target: "tour" },
        { label: "インストール", target: "install" },
        { label: "ドキュメント", target: "docs" },
      ],
      github: "GitHub",
      install: "インストール",
    },
    hero: {
      title: "TOKENUSAGE",
      subtitleLead: "Tokens、制限、アクティビティをひとつの",
      subtitleAccent: "高速ローカルワークフローへ。",
      worksWith: "対応サービス",
      supported: "サポート済み",
      explore: "詳しく見る",
      starOnGithub: "GitHub で Star",
      installBadge: "インストール",
      installLead: "1 行で入れて、そのまま実行",
      tags: {
        faster: {
          label: "214倍高速",
          title: "Rust ネイティブ速度",
          details: [
            "実ログで ccusage より 214 倍高速",
            "Zero-copy JSONL パイプライン",
            "10 万件超でも 1 秒未満で結果を返す",
          ],
        },
        local: {
          label: "100% ローカル",
          title: "ネットワーク呼び出しなし",
          details: [
            "解析はすべて手元のマシンで実行",
            "ログはディスクから出ない",
            "オフラインでも動作、API キー不要",
          ],
        },
        sources: {
          label: "3 ソース",
          title: "統合レポート",
          details: [
            "Claude Code + OpenAI Codex + Antigravity",
            "単一のマージ済みトークンレポート",
            "プロバイダ横断の日次・週次サマリー",
          ],
        },
        live: {
          label: "ライブ監視",
          title: "リアルタイム追跡",
          details: [
            "tu live がコーディング中の使用量を配信",
            "上限に当たる前に警告",
            "heartbeat ベースの作業時間トラッキング",
          ],
        },
        share: {
          label: "共有カード",
          title: "利用状況を見せる",
          details: [
            "美しい PNG カードを生成",
            "tu img day / tu img week",
            "SNS やチーム共有に最適",
          ],
        },
      },
      visualAlts: {
        gui: "tokenusage GUI ダッシュボード",
        cli: "tokenusage CLI レポート",
        shareWeek: "tokenusage 週間共有カード",
        shareDay: "tokenusage 共有カード",
        live: "tokenusage ライブモニター",
      },
    },
    metrics: [
      { value: "0.08s", label: "1,521 ファイル、2.2 GB の Claude ログでもウォーム実行は 0.08 秒" },
      { value: "214x", label: "0.08 秒 vs 17.15 秒、Rust + 並列スキャン + キャッシュ" },
      { value: "3 ソース", label: "Codex、Claude、Antigravity を 1 つのビューに統合" },
      { value: "100% ローカル", label: "ログは手元に残ります。取得するのは価格メタデータだけです。" },
    ],
    tour: {
      badge: "ツアー",
      title: "すべてのコマンドを、ひとつの流れで。",
      description: "各機能をスクロールしながら、tokenusage が AI コーディング全体をどうカバーするか確認できます。",
      panels: {
        daily: { panelTitle: "統合 CLI レポート", stepTitle: "統合トークンレポート", linkLabel: "クイックスタート" },
        live: { panelTitle: "ライブモニター", stepTitle: "リアルタイム TUI モニター", linkLabel: "ライブ docs" },
        top: { panelTitle: "セッショントップ", stepTitle: "トークン版 htop", linkLabel: "top docs" },
        today: { panelTitle: "今日の表示", stepTitle: "今日のコーディング活動", linkLabel: "活動 docs" },
        activity: { panelTitle: "アクティビティ表示", stepTitle: "複数日アクティビティ分析", linkLabel: "活動 docs" },
        heartbeat: { panelTitle: "Heartbeat システム", stepTitle: "ローカル heartbeat コレクター", linkLabel: "Heartbeat docs" },
        img: { panelTitle: "共有カード", stepTitle: "共有できる画像カード", linkLabel: "画像 docs" },
        gui: { panelTitle: "GUI ダッシュボード", stepTitle: "デスクトップ GUI ダッシュボード", linkLabel: "GUI docs" },
        periods: { panelTitle: "期間レポート", stepTitle: "週次・月次レポート", linkLabel: "期間 docs" },
        statusline: {
          panelTitle: "Statusline",
          stepTitle: "エディタと tmux の連携",
          linkLabel: "Statusline docs",
          contentHeading: "ワークフローに埋め込む",
          contentBody:
            "tmux、Neovim、またはシェル出力を読む任意のツール向けにコンパクトな文字列を出力します。セッションコスト、制限、消費ペースを含みます。",
        },
      },
      imageAlts: {
        dailyShare: "日次共有カード",
        weeklyShare: "週次共有カード",
        gui: "GUI ダッシュボードのデモ",
      },
    },
    install: {
      badge: "インストール",
      title: "1 回のインストールで CLI、TUI、GUI。",
      thenRun: "その後に実行",
      commands: [
        { cmd: "tu", tip: "統合トークンレポート" },
        { cmd: "tu live codex", tip: "制限と消費ペースをリアルタイム表示" },
        { cmd: "tu today", tip: "今日のアクティビティ表示" },
        { cmd: "tu img week", tip: "週次共有カードを生成" },
        { cmd: "tu gui", tip: "GUI ダッシュボードを開く" },
        { cmd: "tu heartbeat watch .", tip: "コーディング活動を追跡" },
      ],
    },
    docs: {
      badge: "ドキュメント",
      title: "Docs、ソース、レジストリ。",
      description: "導入、比較、好みのレジストリからのインストールに必要な情報をまとめています。",
      links: [
        {
          label: "クイックスタート",
          title: "コマンド中心の導入",
          desc: "インストールして実行し、ライブ監視、アクティビティ追跡、共有カード生成まで進めます。",
        },
        {
          label: "比較",
          title: "tokenusage vs ccusage",
          desc: "性能、統合レポート、ライブ監視、プロダクト面を比較します。",
        },
        {
          label: "レジストリ",
          title: "Rust crate",
          desc: "crates.io または cargo-binstall から入れて、ローカルで tu を使えます。",
        },
        {
          label: "レジストリ",
          title: "npm パッケージ",
          desc: "1 行で導入したいユーザー向けのグローバルインストール経路です。",
        },
      ],
    },
    footer: {
      badge: "準備完了",
      title: "トークンを追跡し、より多くのコードを届ける。",
      description: "1 回のインストールで必要なトークン指標をすべて取得。高速、ローカル、オープンソース。",
      starOnGithub: "GitHub で Star",
      contact: "お問い合わせ",
      builtWith: "tokenusage · Rust 製",
    },
  },
};

function resolveLocale(input?: string | null): Locale {
  if (!input) return "en";
  const normalized = input.toLowerCase();
  const match = LANGUAGE_OPTIONS.find(({ code }) => normalized === code || normalized.startsWith(`${code}-`));
  return match?.code ?? "en";
}

function readStoredLocale(): Locale | null {
  if (typeof window === "undefined") return null;
  try {
    const stored = window.localStorage.getItem(LOCALE_STORAGE_KEY);
    return stored ? resolveLocale(stored) : null;
  } catch {
    return null;
  }
}

function detectInitialLocale(): Locale {
  const stored = readStoredLocale();
  if (stored) return stored;
  if (typeof navigator === "undefined") return "en";

  for (const candidate of navigator.languages) {
    const locale = resolveLocale(candidate);
    if (locale !== "en" || candidate.toLowerCase().startsWith("en")) {
      return locale;
    }
  }

  return resolveLocale(navigator.language);
}

function updateMeta(selector: string, content: string) {
  const element = document.querySelector(selector);
  if (element) {
    element.setAttribute("content", content);
  }
}

interface I18nContextValue {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  messages: Messages;
}

const I18nContext = createContext<I18nContextValue | null>(null);

export function I18nProvider({ children }: { children: ReactNode }) {
  const [locale, setLocale] = useState<Locale>(detectInitialLocale);
  const messages = TRANSLATIONS[locale];

  useEffect(() => {
    document.documentElement.lang = locale;
    try {
      window.localStorage.setItem(LOCALE_STORAGE_KEY, locale);
    } catch {
      // Ignore storage write failures.
    }
  }, [locale]);

  useEffect(() => {
    document.title = messages.meta.title;
    updateMeta('meta[name="description"]', messages.meta.description);
    updateMeta('meta[property="og:title"]', messages.meta.title);
    updateMeta('meta[property="og:description"]', messages.meta.socialDescription);
    updateMeta('meta[name="twitter:title"]', messages.meta.title);
    updateMeta('meta[name="twitter:description"]', messages.meta.socialDescription);
  }, [messages, locale]);

  return (
    <I18nContext.Provider value={{ locale, setLocale, messages }}>
      {children}
    </I18nContext.Provider>
  );
}

export function useI18n() {
  const context = useContext(I18nContext);
  if (!context) {
    throw new Error("useI18n must be used within I18nProvider");
  }
  return context;
}
