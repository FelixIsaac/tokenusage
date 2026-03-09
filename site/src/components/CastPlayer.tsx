import { memo, useRef, useEffect, useState, useCallback } from "react";
import "asciinema-player/dist/bundle/asciinema-player.css";

const buttonClass =
  "flex items-center gap-2 px-5 py-3 rounded-full border border-cyan/30 bg-[rgba(10,22,38,0.92)] text-cyan font-[family-name:var(--font-display)] text-sm tracking-wider hover:border-cyan/50 hover:-translate-y-0.5 transition-all";

function CastPlayerInner({
  src,
  className,
  active,
  onRequestActivate,
}: {
  src: string;
  className?: string;
  active?: boolean;
  onRequestActivate?: () => void;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const playerRef = useRef<unknown>(null);
  const [ended, setEnded] = useState(false);
  const [playing, setPlaying] = useState(false);
  const hasPlayedRef = useRef(false);

  // Create player on mount with last-frame poster, no autoplay
  useEffect(() => {
    const el = containerRef.current;
    if (!el || playerRef.current) return;

    let cancelled = false;

    import("asciinema-player").then((mod) => {
      if (cancelled || playerRef.current) return;

      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const player = (mod as any).create(src, el, {
        autoPlay: false,
        loop: false,
        speed: 1.5,
        idleTimeLimit: 2,
        fit: "width",
        terminalFontFamily: "'JetBrains Mono', 'Fira Code', 'SF Mono', Menlo, monospace",
        terminalFontSize: "13px",
        theme: "dracula",
        controls: false,
        poster: "npt:99999",
      });

      playerRef.current = player;
      player.addEventListener("ended", () => {
        setEnded(true);
        setPlaying(false);
      });
    });

    return () => {
      cancelled = true;
    };
  }, [src]);

  // Play/pause on active changes
  useEffect(() => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const player = playerRef.current as any;
    if (!player) return;

    if (active) {
      player.seek(0);
      player.play();
      setEnded(false);
      setPlaying(true);
      hasPlayedRef.current = true;
    } else if (hasPlayedRef.current) {
      player.pause();
      setPlaying(false);
    }
  }, [active]);

  const replay = useCallback(() => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const player = playerRef.current as any;
    if (player) {
      player.seek(0);
      player.play();
      setEnded(false);
      setPlaying(true);
    }
  }, []);

  const showPlay = !playing && !ended;
  const showReplay = ended;

  return (
    <div className={`relative overflow-hidden ${className ?? ""}`}>
      <div ref={containerRef} className="w-full h-full [&_.ap-overlay]:!hidden [&_.ap-start-button]:!hidden" />

      {showPlay && (
        <div
          className="absolute inset-0 flex items-center justify-center cursor-pointer rounded-2xl transition-opacity"
          onClick={(e) => {
            e.stopPropagation();
            if (active) {
              replay();
            } else {
              onRequestActivate?.();
            }
          }}
        >
          <div className={buttonClass}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" stroke="none">
              <polygon points="6,3 20,12 6,21" />
            </svg>
            Play
          </div>
        </div>
      )}

      {showReplay && (
        <div
          className="absolute inset-0 flex items-center justify-center bg-[rgba(5,11,20,0.6)] backdrop-blur-[2px] cursor-pointer rounded-2xl transition-opacity"
          onClick={(e) => {
            e.stopPropagation();
            if (!active) onRequestActivate?.();
            else replay();
          }}
        >
          <div className={buttonClass}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <polyline points="1 4 1 10 7 10" />
              <path d="M3.51 15a9 9 0 1 0 2.13-9.36L1 10" />
            </svg>
            Replay
          </div>
        </div>
      )}
    </div>
  );
}

const CastPlayer = memo(CastPlayerInner, (prev, next) =>
  prev.src === next.src && prev.active === next.active && prev.className === next.className
);
export default CastPlayer;
