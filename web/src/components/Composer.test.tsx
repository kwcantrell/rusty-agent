import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Composer } from "./Composer";

const setup = (history: string[] = []) => {
  const onSend = vi.fn();
  render(<Composer disabled={false} onSend={onSend} history={() => history} />);
  const ta = screen.getByRole("textbox", { name: "prompt" }) as HTMLTextAreaElement;
  return { onSend, ta };
};

describe("Composer", () => {
  it("sends on Enter and clears", () => {
    const { onSend, ta } = setup();
    fireEvent.change(ta, { target: { value: "hello" } });
    fireEvent.keyDown(ta, { key: "Enter" });
    expect(onSend).toHaveBeenCalledWith("hello");
    expect(ta.value).toBe("");
  });
  it("does not send on Shift+Enter", () => {
    const { onSend, ta } = setup();
    fireEvent.change(ta, { target: { value: "hello" } });
    fireEvent.keyDown(ta, { key: "Enter", shiftKey: true });
    expect(onSend).not.toHaveBeenCalled();
  });
  it("ArrowUp recalls history newest-first", () => {
    const { ta } = setup(["first", "second"]);
    fireEvent.keyDown(ta, { key: "ArrowUp" });
    expect(ta.value).toBe("second");
    fireEvent.keyDown(ta, { key: "ArrowUp" });
    expect(ta.value).toBe("first");
    fireEvent.keyDown(ta, { key: "ArrowUp" }); // at oldest: stays
    expect(ta.value).toBe("first");
  });
  it("ArrowDown walks forward and restores the draft past the newest", () => {
    const { ta } = setup(["first", "second"]);
    fireEvent.change(ta, { target: { value: "my draft" } });
    fireEvent.keyDown(ta, { key: "ArrowUp" });
    fireEvent.keyDown(ta, { key: "ArrowDown" });
    expect(ta.value).toBe("my draft");
  });
  it("ArrowUp with no history is a no-op", () => {
    const { ta } = setup([]);
    fireEvent.change(ta, { target: { value: "draft" } });
    fireEvent.keyDown(ta, { key: "ArrowUp" });
    expect(ta.value).toBe("draft");
  });
  it("shows the disconnected placeholder when disabled", () => {
    render(<Composer disabled onSend={() => {}} history={() => []} />);
    expect(screen.getByPlaceholderText(/disconnected/)).toBeDisabled();
  });
});
