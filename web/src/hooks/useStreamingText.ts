import { useState, useEffect, useRef } from "react";

const CHARS_PER_SECOND = 60;
// Extra reveal speed proportional to the backlog: when a burst of tokens lands
// the reveal accelerates so it never lags arbitrarily far behind a fast stream.
const CATCHUP_PER_SECOND = 2;
const CURSOR_PERIOD_MS = 530;

/**
 * Returns a progressively revealed version of `text` when `isStreaming` is true.
 * The reveal advances at a steady base rate (~60 chars/sec) plus a catch-up term
 * so it keeps pace with fast/large streams. Appended tokens do NOT restart the
 * reveal — the position is preserved across deltas; it only resets when `text` is
 * replaced by a non-extending value (a different message reusing this component) or
 * shrinks. When `isStreaming` is false, returns the full `text` immediately.
 */
export function useStreamingText(text: string, isStreaming: boolean): string {
  const [revealed, setRevealed] = useState(isStreaming ? "" : text);
  const idxRef = useRef(isStreaming ? 0 : text.length);
  const rafRef = useRef<number | null>(null);
  const lastTsRef = useRef(0);
  const textRef = useRef(text);
  const prevTextRef = useRef(text);

  // Reset the reveal only when the text is replaced (not a pure append) or shrinks;
  // a streamed append keeps the current position so the prefix doesn't flash back.
  if (text !== prevTextRef.current) {
    if (!text.startsWith(prevTextRef.current) || text.length < idxRef.current) {
      idxRef.current = 0;
    }
    prevTextRef.current = text;
  }
  textRef.current = text;

  useEffect(() => {
    if (rafRef.current !== null) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }

    if (!isStreaming) {
      idxRef.current = text.length;
      setRevealed(text);
      return;
    }

    // Reveal advances only inside the rAF tick (async) — never synchronously here,
    // which would re-enter this effect on each render and exhaust the update depth.
    // `revealed` already holds the prefix shown so far, so an append doesn't flash back.
    lastTsRef.current = performance.now();

    const tick = (now: number) => {
      const dt = (now - lastTsRef.current) / 1000;
      lastTsRef.current = now;
      const remaining = textRef.current.length - idxRef.current;
      if (remaining > 0) {
        const rate = CHARS_PER_SECOND + remaining * CATCHUP_PER_SECOND;
        const step = Math.max(1, Math.floor(dt * rate));
        idxRef.current = Math.min(idxRef.current + step, textRef.current.length);
        setRevealed(textRef.current.slice(0, idxRef.current));
      }
      rafRef.current = requestAnimationFrame(tick);
    };
    rafRef.current = requestAnimationFrame(tick);

    return () => {
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    };
  }, [isStreaming, text]);

  return revealed;
}

/**
 * Returns a boolean that toggles every CURSOR_PERIOD_MS for a blinking cursor.
 */
export function useStreamingCursor(): boolean {
  const [visible, setVisible] = useState(true);
  useEffect(() => {
    const id = setInterval(() => setVisible((v) => !v), CURSOR_PERIOD_MS);
    return () => clearInterval(id);
  }, []);
  return visible;
}
