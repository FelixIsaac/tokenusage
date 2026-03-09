import { memo, useRef, useEffect, useState, useCallback } from "react";
import "asciinema-player/dist/bundle/asciinema-player.css";

/**
 * Asciinema cast player with replay overlay.
 *
 * Layout-shift prevention:
 * - The outer container MUST have a fixed height set by the caller
 *   (e.g. via className or style) so IntersectionObserver never fires
 *   from the player mounting.
 * - overflow-hidden clips any player content that exceeds bounds.
 * - React.memo(..., () => true) prevents parent re-renders from
 *   propagating into this component.
 */
function CastPlayerInner({ src, className }: { src: string; className?: string }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const playerRef = useRef<unknown>(null);
  const [ended, setEnded] = useState(false);

  useEffect(() => {
    const el = containerRef.current;
    if (!el || playerRef.current) return;

    let cancelled = false;

    import("asciinema-player").then((mod) => {
      if (cancelled || playerRef.current) return;

      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const player = (mod as any).create(src, el, {
        autoPlay: true,
        loop: false,
        speed: 1.5,
        idleTimeLimit: 2,
        fit: "width",
        terminalFontFamily: "'JetBrains Mono', 'Fira Code', 'SF Mono', Menlo, monospace",
        terminalFontSize: "13px",
        theme: "dracula",
        controls: false,
      });

      playerRef.current = player;
      player.addEventListener("ended", () => setEnded(true));
    });

    return () => { cancelled = true; };
  }, [src]);

  const replay = useCallback(() => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const player = playerRef.current as any;
    if (player) {
      player.seek(0);
      player.play();
      setEnded(false);
    }
  }, []);

  return (
    <div className={`relative overflow-hidden ${className ?? ""}`}>
      <div ref={containerRef} className="w-full h-full" />

      {ended && (
        <div
          className="absolute inset-0 flex items-center justify-center bg-[rgba(5,11,20,0.6)] backdrop-blur-[2px] cursor-pointer rounded-2xl transition-opacity"
          onClick={replay}
        >
          <div className="flex items-center gap-2 px-5 py-3 rounded-full border border-cyan/30 bg-[rgba(10,22,38,0.92)] text-cyan font-[family-name:var(--font-display)] text-sm tracking-wider hover:border-cyan/50 hover:-translate-y-0.5 transition-all">
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

const CastPlayer = memo(CastPlayerInner, () => true);
export default CastPlayer;
