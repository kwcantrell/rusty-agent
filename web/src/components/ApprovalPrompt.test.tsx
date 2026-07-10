import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ApprovalPrompt } from "./ApprovalPrompt";

const approval = { id: "a1", summary: "run `rm -rf node_modules`", command: "rm -rf node_modules" };

describe("ApprovalPrompt", () => {
  it("renders numbered options and decides on click", () => {
    const onDecide = vi.fn();
    render(<ApprovalPrompt approval={approval} onDecide={onDecide} />);
    fireEvent.click(screen.getByText(/Yes, don't ask again/));
    expect(onDecide).toHaveBeenCalledWith("approve_always");
  });
  it("maps keys 1/2/3 to approve/approve_always/deny", () => {
    const onDecide = vi.fn();
    render(<ApprovalPrompt approval={approval} onDecide={onDecide} />);
    fireEvent.keyDown(window, { key: "3" });
    expect(onDecide).toHaveBeenCalledWith("deny");
  });
  it("ignores digit keys typed into a textarea", () => {
    const onDecide = vi.fn();
    render(
      <>
        <textarea aria-label="prompt" />
        <ApprovalPrompt approval={approval} onDecide={onDecide} />
      </>,
    );
    fireEvent.keyDown(screen.getByLabelText("prompt"), { key: "1" });
    expect(onDecide).not.toHaveBeenCalled();
  });
  it("deny with feedback sends the object decision", () => {
    const onDecide = vi.fn();
    render(<ApprovalPrompt approval={approval} onDecide={onDecide} />);
    fireEvent.change(screen.getByPlaceholderText(/optional feedback/i), {
      target: { value: "use staging" },
    });
    fireEvent.click(screen.getByText(/^3\./).closest("button") ?? screen.getByText("No"));
    expect(onDecide).toHaveBeenCalledWith({ deny: { feedback: "use staging" } });
  });

  it("deny with empty feedback sends the legacy string", () => {
    const onDecide = vi.fn();
    render(<ApprovalPrompt approval={approval} onDecide={onDecide} />);
    fireEvent.keyDown(window, { key: "3" });
    expect(onDecide).toHaveBeenCalledWith("deny");
  });
});
