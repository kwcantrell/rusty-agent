import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StatusBar } from "../src/components/StatusBar";
import { MessageList } from "../src/components/MessageList";
import { ApprovalPrompt } from "../src/components/ApprovalPrompt";
import { Composer } from "../src/components/Composer";

describe("shell components", () => {
  it("StatusBar shows presence and triggers sign-out", async () => {
    const onSignOut = vi.fn();
    render(<StatusBar online={true} status="open" onSignOut={onSignOut} />);
    expect(screen.getByText(/online/i)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /sign out/i }));
    expect(onSignOut).toHaveBeenCalled();
  });

  it("MessageList renders items in order by type", () => {
    render(<MessageList items={[
      { kind: "user", text: "hi" },
      { kind: "assistant", text: "hello", done: "stop" },
      { kind: "error", message: "boom" },
    ]} />);
    expect(screen.getByText("hi")).toBeInTheDocument();
    expect(screen.getByText("hello")).toBeInTheDocument();
    expect(screen.getByText(/boom/)).toBeInTheDocument();
  });

  it("ApprovalPrompt emits the chosen decision", async () => {
    const onDecide = vi.fn();
    render(<ApprovalPrompt approval={{ id: "c0", summary: "run x", command: "x" }} onDecide={onDecide} />);
    await userEvent.click(screen.getByRole("button", { name: /^approve$/i }));
    expect(onDecide).toHaveBeenCalledWith("approve");
  });

  it("Composer sends text and is disabled when offline", async () => {
    const onSend = vi.fn();
    const { rerender } = render(<Composer disabled={false} onSend={onSend} />);
    await userEvent.type(screen.getByRole("textbox"), "do it");
    await userEvent.click(screen.getByRole("button", { name: /send/i }));
    expect(onSend).toHaveBeenCalledWith("do it");
    rerender(<Composer disabled={true} onSend={onSend} />);
    expect(screen.getByRole("button", { name: /send/i })).toBeDisabled();
  });
});
