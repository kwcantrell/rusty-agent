import { routeFor } from "./router";
import { formatStats, type RawStats } from "./stats";

async function render(): Promise<void> {
  const app = document.querySelector<HTMLDivElement>("#app")!;
  const path = location.hash.replace(/^#/, "") || "/";
  const page = routeFor(path);
  if (!page) {
    app.innerHTML = `<h1>Page Not Found</h1>`;
    return;
  }
  let extra = "";
  if (path === "/stats") {
    const raw = (await (await fetch("/stats.json")).json()) as RawStats;
    const v = formatStats(raw);
    extra = `<ul><li>${v.dailyActive}</li><li>${v.latencyP95}</li><li>${v.uptime}</li></ul>`;
  }
  app.innerHTML = `<h1>${page.title}</h1><p>${page.body}</p>${extra}`;
}

window.addEventListener("hashchange", render);
void render();
