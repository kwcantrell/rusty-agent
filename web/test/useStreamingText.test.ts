import { describe, it, expect, vi, afterEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useStreamingText, useStreamingCursor } from "../src/hooks/useStreamingText";

afterEach(() => {
  vi.useRealTimers();
});

describe("useStreamingText", () => {
  it("returns full text when not streaming", () => {
    const { result } = renderHook(() => useStreamingText("hello", false));
    expect(result.current).toBe("hello");
  });

  it("returns empty string for empty text", () => {
    const { result } = renderHook(() => useStreamingText("", false));
    expect(result.current).toBe("");
  });

  it("reveals characters incrementally while streaming", () => {
    vi.useFakeTimers();
    const { result } = renderHook(
      ({ text, isStreaming }) => useStreamingText(text, isStreaming),
      { initialProps: { text: "hello", isStreaming: true } }
    );

    // At t=0, no chars revealed yet
    expect(result.current).toBe("");

    // After 1ms steps, chars should be revealed at ~60 chars/sec
    // 1000 steps of 1ms = 1000ms = 1 second, which should reveal all 5 chars
    act(() => {
      for (let i = 0; i < 1000; i++) vi.advanceTimersByTime(1);
    });
    expect(result.current).toBe("hello");
  });

  it("resets index when text changes while streaming", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(
      ({ text, isStreaming }) => useStreamingText(text, isStreaming),
      { initialProps: { text: "ab", isStreaming: true } }
    );

    act(() => {
      for (let i = 0; i < 1000; i++) vi.advanceTimersByTime(1);
    });
    expect(result.current).toBe("ab");

    // Change text while still streaming
    rerender({ text: "cd", isStreaming: true });
    act(() => {
      for (let i = 0; i < 1000; i++) vi.advanceTimersByTime(1);
    });
    expect(result.current).toBe("cd");
  });

  it("continues from the current position when text is appended (does not restart)", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(
      ({ text, isStreaming }) => useStreamingText(text, isStreaming),
      { initialProps: { text: "Hello", isStreaming: true } }
    );

    // Fully reveal the first chunk.
    act(() => {
      for (let i = 0; i < 1000; i++) vi.advanceTimersByTime(1);
    });
    expect(result.current).toBe("Hello");

    // Append more text — the already-revealed prefix must NOT flash back to the start.
    rerender({ text: "Hello world", isStreaming: true });
    act(() => {
      vi.advanceTimersByTime(16);
    });
    expect(result.current.startsWith("Hello")).toBe(true);
    expect(result.current.length).toBeGreaterThanOrEqual("Hello".length);
  });

  it("switches to full text when isStreaming flips to false", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(
      ({ text, isStreaming }) => useStreamingText(text, isStreaming),
      { initialProps: { text: "hello world", isStreaming: true } }
    );

    // Not all chars revealed yet — advance only a bit
    act(() => {
      vi.advanceTimersByTime(50);
    });
    const partial = result.current;
    expect(partial.length).toBeLessThan("hello world".length);

    // Stop streaming
    rerender({ text: "hello world", isStreaming: false });
    expect(result.current).toBe("hello world");
  });
});

describe("useStreamingCursor", () => {
  it("toggles between true and false", () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useStreamingCursor());
    const first = result.current;
    act(() => {
      vi.advanceTimersByTime(530);
    });
    expect(result.current).toBe(!first);
  });
});
