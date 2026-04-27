import maplibregl from "maplibre-gl";
import { GeomqttSource } from "@openfantasymap/geomqtt-maplibre";
import { fetchServerConfig } from "@openfantasymap/geomqtt-core";

/** Map a `ws[s]://host:port/<path>` to the matching `http[s]://host:port`. */
function deriveHttpUrl(wsUrl: string): string {
  const u = new URL(wsUrl);
  u.protocol = u.protocol === "wss:" ? "https:" : "http:";
  u.pathname = "/";
  return u.toString().replace(/\/$/, "");
}

type Projection = "globe" | "mercator";

const params = new URLSearchParams(location.search);
const initialUrl = params.get("url") ?? "";
const initialSet = params.get("set") ?? "iss";
const initialProj: Projection = params.get("proj") === "mercator" ? "mercator" : "globe";

const urlInput = document.getElementById("url") as HTMLInputElement;
const setInput = document.getElementById("set") as HTMLInputElement;
const button = document.getElementById("connect") as HTMLButtonElement;
const status = document.getElementById("status") as HTMLDivElement;
const topicsPanel = document.getElementById("topics") as HTMLDivElement;
const topicsCount = document.getElementById("topics-count") as HTMLSpanElement;
const activeList = document.getElementById("active") as HTMLDivElement;
const logList = document.getElementById("log") as HTMLDivElement;

const activeTopics = new Set<string>();
const MAX_LOG = 30;

function renderActive(): void {
  topicsCount.textContent = String(activeTopics.size);
  const sorted = Array.from(activeTopics).sort();
  activeList.replaceChildren(
    ...sorted.map((t) => {
      const el = document.createElement("div");
      el.textContent = t;
      return el;
    }),
  );
}

function pushLog(op: "add" | "rm", topics: string[]): void {
  const ts = new Date().toLocaleTimeString(undefined, { hour12: false });
  for (const topic of topics) {
    const row = document.createElement("div");
    row.className = op;
    row.innerHTML = `<span class="t">${ts}</span><span class="op">${op === "add" ? "+" : "−"}</span><span class="topic"></span>`;
    (row.querySelector(".topic") as HTMLSpanElement).textContent = topic;
    logList.prepend(row);
  }
  while (logList.childElementCount > MAX_LOG) {
    logList.lastElementChild?.remove();
  }
}

urlInput.value = initialUrl;
setInput.value = initialSet;

const map = new maplibregl.Map({
  container: "map",
  style: "https://demotiles.maplibre.org/style.json",
  center: [0, 20],
  zoom: 1.5,
  hash: false,
});

map.addControl(new maplibregl.NavigationControl(), "top-right");

const projButtons = Array.from(
  document.querySelectorAll<HTMLButtonElement>(".proj-btn"),
);

function setProjection(proj: Projection): void {
  map.setProjection({ type: proj });
  for (const b of projButtons) {
    b.classList.toggle("active", b.dataset.proj === proj);
  }
  const loc = new URL(location.href);
  if (proj === "globe") loc.searchParams.delete("proj");
  else loc.searchParams.set("proj", proj);
  history.replaceState(null, "", loc.toString());
}

for (const b of projButtons) {
  b.addEventListener("click", () => setProjection(b.dataset.proj as Projection));
}

map.once("load", () => setProjection(initialProj));

let source: GeomqttSource | null = null;

function setStatus(text: string, error = false): void {
  status.textContent = text;
  status.classList.toggle("error", error);
}

async function waitForMapLoad(): Promise<void> {
  if (map.isStyleLoaded()) return;
  await new Promise<void>((resolve) => map.once("load", () => resolve()));
}

async function connect(url: string, set: string): Promise<void> {
  if (!url) {
    setStatus("Please enter a wss:// URL.", true);
    return;
  }
  button.disabled = true;
  setStatus(`Connecting to ${url} …`);
  try {
    if (source) {
      source.detach();
      source = null;
    }
    activeTopics.clear();
    logList.replaceChildren();
    renderActive();
    topicsPanel.classList.remove("hidden");
    await waitForMapLoad();

    // Read the server's effective zoom list so we subscribe to exactly
    // what it publishes. Falls back to the client default if /config is
    // unreachable (e.g. an older server without CORS).
    let publishedZooms: number[] | undefined;
    try {
      const cfg = await fetchServerConfig(deriveHttpUrl(url));
      publishedZooms = cfg.zooms;
    } catch {
      publishedZooms = undefined;
    }

    source = new GeomqttSource({
      map,
      url,
      set,
      publishedZooms,
      updateThrottleMs: 500,
      layers: [
        {
          id: `geomqtt-${set}-halo`,
          type: "circle",
          source: `geomqtt-${set}`,
          paint: {
            "circle-radius": 14,
            "circle-color": "#ff5722",
            "circle-opacity": 0.25,
          },
        },
        {
          id: `geomqtt-${set}-dot`,
          type: "circle",
          source: `geomqtt-${set}`,
          paint: {
            "circle-radius": 6,
            "circle-color": "#ff5722",
            "circle-stroke-color": "#fff",
            "circle-stroke-width": 2,
          },
        },
      ],
    });
    source.client.on((ev) => {
      if (ev.type === "subscribed") {
        for (const t of ev.topics) activeTopics.add(t);
        renderActive();
        pushLog("add", ev.topics);
      } else if (ev.type === "unsubscribed") {
        for (const t of ev.topics) activeTopics.delete(t);
        renderActive();
        pushLog("rm", ev.topics);
      }
    });
    await source.attach();
    setStatus(`Connected — set "${set}". Pan/zoom to subscribe to tiles.`);
  } catch (err: unknown) {
    const message = err instanceof Error ? err.message : String(err);
    setStatus(`Error: ${message}`, true);
  } finally {
    button.disabled = false;
  }
}

button.addEventListener("click", () => {
  const url = urlInput.value.trim();
  const set = setInput.value.trim() || "iss";
  const loc = new URL(location.href);
  loc.searchParams.set("url", url);
  loc.searchParams.set("set", set);
  history.replaceState(null, "", loc.toString());
  void connect(url, set);
});

if (initialUrl) {
  void connect(initialUrl, initialSet);
}
