import type { Theme } from "../theme";

export function ThemeToggle({ theme, onToggle }: { theme: Theme; onToggle: () => void }) {
  return (
    <button
      onClick={onToggle}
      aria-label="toggle theme"
      title={theme === "dark" ? "Switch to light" : "Switch to dark"}
      style={{ color: "var(--text-muted)" }}
      className="hover:opacity-80"
    >
      {theme === "dark" ? "◐" : "◑"}
    </button>
  );
}
