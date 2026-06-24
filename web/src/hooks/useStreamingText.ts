import { useState, useEffect, useRef, useCallback } from "react";

const CHARS_PER_SECOND = 60;
const CURSOR_PERIOD_MS = 530;

/**
 * Returns a progressively revealed version of `text` when `isStreaming` is true.
 * While streaming, characters are revealed at ~60 chars/sec based on elapsed time.
 * When `isStreaming` is false, returns the full `text` immediately.
 * When `text` changes, the reveal index resets to 0.
 */
export function useStreamingText(text: string, isStreaming: boolean): string {
  const [revealed, setRevealed] = useState("");
  const idxRef = useRef(0);
  const rafRef = useRef<number | null>(null);
  const textRef = useRef(text);
  const streamingRef = useRef(isStreaming);
  const startTimeRef = useRef(0);

  textRef.current = text;
  streamingRef.current = isStreaming;

  // On first render, show full text when not streaming
  if (revealed === "" && !isStreaming && text.length > 0) {
    setRevealed(text);
  }

  const tick = useCallback(() => {
    if (!streamingRef.current) {
      rafRef.current = null;
      return;
    }
    const target = Math.min(
      Math.floor(((performance.now() - startTimeRef.current) / 1000) * CHARS_PER_SECOND),
      textRef.current.length,
    );
    if (target > idxRef.current) {
      idxRef.current = target;
      setRevealed(textRef.current.slice(0, idxRef.current));
    }
    rafRef.current = requestAnimationFrame(tick);
  }, []);

  useEffect(() => {
    if (rafRef.current !== null) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }

    if (isStreaming && text.length > 0) {
      startTimeRef.current = performance.now();
      idxRef.current = 0;
      rafRef.current = requestAnimationFrame(tick);
    } else {
      setRevealed(text);
    }

    return () => {
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    };
  }, [isStreaming, text, tick]);

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
