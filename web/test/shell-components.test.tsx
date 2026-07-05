import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ApprovalPrompt } from "../src/components/ApprovalPrompt";
import { Composer } from "../src/components/Composer";

// StatusBar coverage moved to TopBar.test.tsx (StatusBar removed in the two-pane redesign).
// MessageList coverage moved to message-list.test.tsx (now takes AnimatedItem[]).
describe("shell components", () => {
  it("ApprovalPrompt emits the chosen decision", async () => {
    const onDecide = vi.fn();
    render(<ApprovalPrompt approval={{ id: "c0", summary: "run x", command: "x" }} onDecide={onDecide} />);
    await userEvent.click(screen.getByRole("button", { name: /^approve$/i }));
    expect(onDecide).toHaveBeenCalledWith("approve");
  });

  it("Composer sends text and is disabled when offline", async () => {
    const onSend = vi.fn();
    const { rerender } = render(<Composer disabled={false} onSend={onSend} history={() => []} />);
    const ta = screen.getByRole("textbox", { name: "prompt" });
    fireEvent.change(ta, { target: { value: "do it" } });
    fireEvent.keyDown(ta, { key: "Enter" });
    expect(onSend).toHaveBeenCalledWith("do it");
    rerender(<Composer disabled={true} onSend={onSend} history={() => []} />);
    expect(screen.getByPlaceholderText(/disconnected/)).toBeDisabled();
  });

  it("Composer submits on Enter but not on Shift+Enter", async () => {
    const onSend = vi.fn();
    render(<Composer disabled={false} onSend={onSend} history={() => []} />);
    const box = screen.getByRole("textbox", { name: "prompt" });
    await userEvent.type(box, "via enter{Enter}");
    expect(onSend).toHaveBeenCalledWith("via enter");
    onSend.mockClear();
    await userEvent.type(box, "no submit{Shift>}{Enter}{/Shift}");
    expect(onSend).not.toHaveBeenCalled();
  });

  it("Composer trims text and ignores whitespace-only input", async () => {
    const onSend = vi.fn();
    render(<Composer disabled={false} onSend={onSend} history={() => []} />);
    const box = screen.getByRole("textbox", { name: "prompt" });
    await userEvent.type(box, "  trimmed  ");
    fireEvent.keyDown(box, { key: "Enter" });
    expect(onSend).toHaveBeenCalledWith("trimmed");
    onSend.mockClear();
    await userEvent.type(box, "   ");
    fireEvent.keyDown(box, { key: "Enter" });
    expect(onSend).not.toHaveBeenCalled();
  });

});
