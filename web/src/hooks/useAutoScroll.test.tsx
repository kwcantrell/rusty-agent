import { render, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { useAutoScroll } from "./useAutoScroll";

// jsdom has no ResizeObserver and no layout; stub the observer and pin the
// geometry so the hook's pinned/scroll math runs against real numbers.
let fireResize: (() => void) | null = null;
class MockResizeObserver {
  constructor(cb: ResizeObserverCallback) {
    fireResize = () => cb([], this as unknown as ResizeObserver);
  }
  observe() {}
  unobserve() {}
  disconnect() {}
}

function Harness() {
  const { containerRef, contentRef, onScroll } = useAutoScroll<HTMLDivElement, HTMLDivElement>();
  return (
    <div data-testid="scroller" ref={containerRef} onScroll={onScroll}>
      <div ref={contentRef} />
    </div>
  );
}

function layout(el: HTMLElement, scrollHeight: number, clientHeight: number) {
  Object.defineProperty(el, "scrollHeight", { configurable: true, value: scrollHeight });
  Object.defineProperty(el, "clientHeight", { configurable: true, value: clientHeight });
}

beforeEach(() => vi.stubGlobal("ResizeObserver", MockResizeObserver));
afterEach(() => {
  vi.unstubAllGlobals();
  fireResize = null;
});

describe("useAutoScroll", () => {
  it("pins to the bottom when content grows", () => {
    const el = render(<Harness />).getByTestId("scroller");
    layout(el, 1000, 200);
    fireResize!();
    expect(el.scrollTop).toBe(1000);
  });

  it("does not yank the user back down after they scroll up", () => {
    const el = render(<Harness />).getByTestId("scroller");
    layout(el, 1000, 200);
    el.scrollTop = 100;
    fireEvent.scroll(el); // 1000 - 100 - 200 = 700 from bottom: unpinned
    fireResize!();
    expect(el.scrollTop).toBe(100);
  });

  it("re-pins when the user scrolls back near the bottom", () => {
    const el = render(<Harness />).getByTestId("scroller");
    layout(el, 1000, 200);
    el.scrollTop = 100;
    fireEvent.scroll(el);
    el.scrollTop = 790;
    fireEvent.scroll(el); // 10px from bottom: within the 40px threshold
    fireResize!();
    expect(el.scrollTop).toBe(1000);
  });
});
