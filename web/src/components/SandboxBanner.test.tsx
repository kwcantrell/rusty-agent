import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { SandboxBanner } from "./SandboxBanner";

describe("SandboxBanner", () => {
  it("shows mechanism and reason and warns about host execution", () => {
    render(<SandboxBanner info={{ mechanism: "docker", reason: "no daemon" }} onDismiss={() => {}} />);
    expect(screen.getByRole("alert").textContent).toMatch(/unsandboxed/i);
    expect(screen.getByRole("alert").textContent).toMatch(/docker/);
    expect(screen.getByRole("alert").textContent).toMatch(/no daemon/);
  });

  it("calls onDismiss when the dismiss control is clicked", () => {
    const onDismiss = vi.fn();
    render(<SandboxBanner info={{ mechanism: "docker", reason: "no daemon" }} onDismiss={onDismiss} />);
    fireEvent.click(screen.getByRole("button", { name: /dismiss/i }));
    expect(onDismiss).toHaveBeenCalledOnce();
  });
});
