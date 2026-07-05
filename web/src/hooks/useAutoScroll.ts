import { useEffect, useRef } from "react";

/** How close to the bottom (px) still counts as "at the bottom". */
const PIN_THRESHOLD_PX = 40;

/**
 * Keeps a scroll container pinned to its bottom as its content grows — new
 * transcript items and streaming text reveal alike (a ResizeObserver on the
 * content wrapper sees both) — without fighting the user: scrolling up to
 * read history unpins; scrolling back to within PIN_THRESHOLD_PX re-pins.
 *
 * Attach `containerRef` + `onScroll` to the scrolling element and
 * `contentRef` to a single wrapper around its content.
 */
export function useAutoScroll<C extends HTMLElement, W extends HTMLElement>() {
  const containerRef = useRef<C | null>(null);
  const contentRef = useRef<W | null>(null);
  const pinned = useRef(true);

  const onScroll = () => {
    const el = containerRef.current;
    if (!el) return;
    pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight <= PIN_THRESHOLD_PX;
  };

  useEffect(() => {
    const el = containerRef.current;
    const content = contentRef.current;
    if (!el || !content || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => {
      if (pinned.current) el.scrollTop = el.scrollHeight;
    });
    ro.observe(content);
    return () => ro.disconnect();
  }, []);

  return { containerRef, contentRef, onScroll };
}
