// Player test UI. Talks to Rust only through typed commands; state arrives as `player-state`.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const SOURCES = [
  ["HLS — Apple bipbop (multi-audio)", "https://devstreaming-cdn.apple.com/videos/streaming/examples/img_bipbop_adv_example_fmp4/master.m3u8"],
  ["MP4 — H.264 720p, 5.1 audio", "https://archive.org/download/BigBuckBunny_124/Content/big_buck_bunny_720p_surround.mp4"],
  ["MKV — Matroska test5 (multi-audio, subtitles)", "https://github.com/ietf-wg-cellar/matroska-test-files/raw/master/test_files/test5.mkv"],
];

const $ = (id) => document.getElementById(id);
const video = $("video");
let state = null;
let active = false;
let selftest = false;
let seeking = false;

video.classList.add("idle");

for (const [label, url] of SOURCES) {
  const button = document.createElement("button");
  button.textContent = label;
  button.onclick = () => open(url);
  $("sources").append(button);
}
$("custom").onsubmit = (event) => {
  event.preventDefault();
  const url = $("customUrl").value.trim();
  if (url) open(url);
};

async function open(url) {
  return play(() => invoke("player_open", { request: { url } }));
}

// Plays through Rust; stream URLs from addons never reach this page.
async function play(start) {
  try {
    await start();
    active = true;
    video.classList.remove("idle");
    $("controls").hidden = false;
    syncRect();
  } catch (error) {
    alert(`Failed to open: ${error}`);
  }
}

// Tells Rust where the video hole is, in CSS pixels, whenever the layout changes.
function syncRect() {
  if (!active || selftest) return;
  const fullscreen = document.body.classList.contains("fullscreen");
  const r = video.getBoundingClientRect();
  const rect = fullscreen ? null : { x: r.left, y: r.top, width: r.width, height: r.height };
  invoke("player_video_rect", { rect, scale: window.devicePixelRatio }).catch(() => {});
}
new ResizeObserver(syncRect).observe(video);
video.addEventListener("transitionend", syncRect);
window.addEventListener("resize", syncRect);

function fmt(seconds) {
  const s = Math.max(0, Math.floor(seconds || 0));
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), r = s % 60;
  return (h ? `${h}:${String(m).padStart(2, "0")}` : `${m}`) + `:${String(r).padStart(2, "0")}`;
}

function fillTracks(select, kind, selectedId) {
  const tracks = state.tracks.filter((t) => t.kind === kind);
  const key = JSON.stringify(tracks.map((t) => [t.id, t.title, t.lang])) + selectedId;
  if (select.dataset.key === key) return;
  select.dataset.key = key;
  select.replaceChildren();
  const off = new Option(kind === "audio" ? "No audio" : "No subtitles", "no");
  select.append(off);
  for (const t of tracks) {
    const label = [t.title, t.lang, t.codec].filter(Boolean).join(" · ") || `#${t.id}`;
    select.append(new Option(`${t.external ? "📄 " : ""}${label}`, String(t.id)));
  }
  select.value = selectedId && selectedId !== "no" ? selectedId : "no";
}

listen("player-state", ({ payload }) => {
  state = payload;
  if (!seeking && state.duration > 0) $("seek").value = String(Math.round((state.timePos / state.duration) * 1000));
  $("time").textContent = `${fmt(state.timePos)} / ${fmt(state.duration)}`;
  $("play").textContent = state.paused ? "▶" : "⏸";
  $("volume").value = String(Math.round(state.volume));
  fillTracks($("audio"), "audio", state.aid);
  fillTracks($("subs"), "sub", state.sid);
  const stats = {
    Decoding: state.hwdecCurrent || "software",
    Codec: state.videoCodec || "—",
    Resolution: state.width ? `${state.width}×${state.height}` : "—",
    FPS: state.fps ? state.fps.toFixed(2) : "—",
    "Dropped frames": `${state.droppedFrames} / ${state.decoderDroppedFrames}`,
    Cache: `${state.cacheSeconds.toFixed(1)} s`,
    State: state.error ? `error: ${state.error}` : state.buffering ? "loading" : state.ended ? "ended" : state.paused ? "paused" : "playing",
  };
  $("stats").replaceChildren(...Object.entries(stats).flatMap(([k, v]) => {
    const dt = document.createElement("dt"); dt.textContent = k;
    const dd = document.createElement("dd"); dd.textContent = v;
    return [dt, dd];
  }));
});

const command = (cmd) => invoke("player_command", { command: cmd }).catch(() => {});
$("play").onclick = () => command({ type: "togglePause" });
$("seek").oninput = () => { seeking = true; };
$("seek").onchange = () => {
  seeking = false;
  if (state?.duration) command({ type: "seek", seconds: ($("seek").value / 1000) * state.duration, relative: false });
};
$("volume").oninput = () => command({ type: "setVolume", volume: Number($("volume").value) });
$("audio").onchange = () => command({ type: "setAudioTrack", id: $("audio").value });
$("subs").onchange = () => command({ type: "setSubtitleTrack", id: $("subs").value });

async function setFullscreen(on) {
  document.body.classList.toggle("fullscreen", on);
  await invoke("window_set_fullscreen", { fullscreen: on });
  syncRect();
}
$("fs").onclick = () => setFullscreen(!document.body.classList.contains("fullscreen"));
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && document.body.classList.contains("fullscreen")) setFullscreen(false);
  if (event.key === " " && active) { event.preventDefault(); command({ type: "togglePause" }); }
});
$("close").onclick = async () => {
  await invoke("player_close");
  active = false;
  video.classList.add("idle");
  $("controls").hidden = true;
  if (document.body.classList.contains("fullscreen")) setFullscreen(false);
};

// Self-test: Rust drives the player; the UI only mirrors the video rect and shows the overlay.
listen("selftest", () => {
  selftest = true;
  document.body.classList.add("selftest");
  video.classList.remove("idle");
  $("controls").hidden = false;
});
listen("selftest-source", ({ payload }) => {
  $("badge").hidden = false;
  $("badge").textContent = `Overlay WebView2 · ${payload}`;
});
listen("selftest-rect", ({ payload }) => {
  const dpr = window.devicePixelRatio;
  if (!payload) {
    Object.assign(video.style, { top: "0px", left: "0px", right: "auto", width: "100vw", height: "100vh", aspectRatio: "auto", borderRadius: "0" });
    $("side").style.display = "none";
  } else {
    const [x, y, w, h] = payload.map((v) => v / dpr);
    Object.assign(video.style, { top: `${y}px`, left: `${x}px`, right: "auto", width: `${w}px`, height: `${h}px`, aspectRatio: "auto", borderRadius: "14px" });
    $("side").style.display = "";
  }
});

// Addon streams for one title: searched only when the user asks.
$("addonForm").onsubmit = async (event) => {
  event.preventDefault();
  const id = $("addonId").value.trim();
  if (!id) return;
  const box = $("addonStreams");
  box.textContent = "Searching…";
  try {
    const groups = await invoke("streams_search", { contentType: $("addonType").value, id, profile: null });
    box.replaceChildren();
    for (const group of groups) {
      const title = document.createElement("div");
      title.textContent = group.error ? `${group.addonName}: ${group.error}` : `${group.addonName} (${group.streams.length})`;
      box.append(title);
      for (const stream of group.streams) {
        const b = document.createElement("button");
        const headers = stream.headerNames.length ? ` · headers: ${stream.headerNames.join(", ")}` : "";
        b.textContent = `${stream.name || "Stream"} — ${stream.description || ""} [${stream.kind}]${headers}`;
        b.disabled = stream.kind !== "direct";
        b.onclick = () => play(() => invoke("stream_play", { key: stream.key, startSeconds: null }));
        box.append(b);
      }
    }
    if (!groups.length) box.textContent = "No addon provides streams for this title.";
  } catch (error) {
    box.textContent = `Failed: ${error}`;
  }
};

$("speed").onchange = () => command({ type: "setSpeed", speed: Number($("speed").value) });
$("mode").onchange = () => command({ type: "setResizeMode", mode: $("mode").value });
$("subDelayMinus").onclick = () => command({ type: "setSubtitleDelay", ms: (state?.subDelayMs || 0) - 250 });
$("subDelayPlus").onclick = () => command({ type: "setSubtitleDelay", ms: (state?.subDelayMs || 0) + 250 });
$("audioDelayMinus").onclick = () => command({ type: "setAudioDelay", ms: (state?.audioDelayMs || 0) - 25 });
$("audioDelayPlus").onclick = () => command({ type: "setAudioDelay", ms: (state?.audioDelayMs || 0) + 25 });
let inPip = false;
$("pip").onclick = () => invoke("player_picture_in_picture", { enabled: !inPip, title: "Nuvio NextGen" }).catch((e) => alert(e));
listen("player-pip", ({ payload }) => { inPip = payload; });
