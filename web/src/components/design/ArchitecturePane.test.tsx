import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { archFixture } from "./archFixture";

const fetchMock = vi.hoisted(() => ({ fn: vi.fn() }));
vi.mock("./architecture", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  fetchArchitecture: fetchMock.fn,
}));

import { ArchitecturePane } from "./ArchitecturePane";

describe("ArchitecturePane", () => {
  beforeEach(() => fetchMock.fn.mockReset());

  it("shows loading then the diagram with the loop block pre-selected", async () => {
    fetchMock.fn.mockResolvedValue(archFixture);
    render(<ArchitecturePane />);
    expect(screen.getByText(/Loading architecture/)).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId("arch-diagram")).toBeInTheDocument());
    expect(screen.getByRole("button", { name: /Agent Loop/ })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("arch-detail")).toBeInTheDocument();
  });

  it("drills down on block click", async () => {
    fetchMock.fn.mockResolvedValue(archFixture);
    render(<ArchitecturePane />);
    await waitFor(() => screen.getByTestId("arch-diagram"));
    fireEvent.click(screen.getByRole("button", { name: /Tools/ }));
    expect(screen.getByText("Render an artifact")).toBeInTheDocument();
  });

  it("error shows retry which refetches", async () => {
    fetchMock.fn.mockRejectedValueOnce(new Error("daemon gone"));
    fetchMock.fn.mockResolvedValueOnce(archFixture);
    render(<ArchitecturePane />);
    await waitFor(() => expect(screen.getByText(/daemon gone/)).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: /Retry/ }));
    await waitFor(() => expect(screen.getByTestId("arch-diagram")).toBeInTheDocument());
    expect(fetchMock.fn).toHaveBeenCalledTimes(2);
  });

  it("refresh button refetches", async () => {
    fetchMock.fn.mockResolvedValue(archFixture);
    render(<ArchitecturePane />);
    await waitFor(() => screen.getByTestId("arch-diagram"));
    fireEvent.click(screen.getByRole("button", { name: /Refresh/ }));
    expect(fetchMock.fn).toHaveBeenCalledTimes(2);
    await waitFor(() => screen.getByTestId("arch-diagram"));
  });

  it("ignores fetch settlement after unmount (no window access post-teardown)", async () => {
    let resolve!: (v: unknown) => void;
    fetchMock.fn.mockReturnValue(new Promise((r) => { resolve = r; }));
    const { unmount } = render(<ArchitecturePane />);
    unmount();
    // Simulate the test environment being torn down before the fetch settles:
    // any React state dispatch would touch `window` and reject unhandled.
    vi.stubGlobal("window", undefined);
    try {
      resolve(archFixture);
      await new Promise((r) => setTimeout(r, 0));
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
