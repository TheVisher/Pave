import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";

interface WindowSlot {
  window_class: string;
  launch_command: string | null;
  monitor: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

interface Preset {
  name: string;
  slots: WindowSlot[];
}

interface PaveConfig {
  gap_size: number;
  excluded_monitors: string[];
  autostart: boolean;
  corner_radius: number | null;
  presets: Preset[];
  restore_session: boolean;
  auto_surface_tabs: boolean;
}

let currentConfig: PaveConfig = {
  gap_size: 15,
  excluded_monitors: [],
  autostart: false,
  corner_radius: null,
  presets: [],
  restore_session: false,
  auto_surface_tabs: true,
};

// Snapshot for dirty tracking
let configSnapshot: string = "";

// ─── DOM helpers ───
const $ = (id: string) => document.getElementById(id)!;
const gapSlider = () => $("gap-slider") as HTMLInputElement;
const gapValue = () => $("gap-value");
const cornerSlider = () => $("corner-slider") as HTMLInputElement;
const cornerValue = () => $("corner-value");
const monitorsListEl = () => $("monitors-list");
const autoSurfaceToggle = () => $("auto-surface-toggle") as HTMLInputElement;
const restoreSessionToggle = () => $("restore-session-toggle") as HTMLInputElement;
const autostartToggle = () => $("autostart-toggle") as HTMLInputElement;
const presetsListEl = () => $("presets-list");
const capturePresetBtn = () => $("capture-preset-btn");
const saveBtn = () => $("save-btn");
const saveBar = () => $("save-bar");

// ─── Navigation ───
const sectionsWithSave = new Set(["appearance", "behavior"]);

function setupNavigation() {
  const navItems = document.querySelectorAll<HTMLElement>(".nav-item");
  navItems.forEach((item) => {
    item.addEventListener("click", () => {
      const section = item.dataset.section!;
      // Update active nav
      navItems.forEach((n) => n.classList.remove("active"));
      item.classList.add("active");
      // Show target section
      document.querySelectorAll(".section").forEach((s) => s.classList.add("hidden"));
      $(`section-${section}`).classList.remove("hidden");
      // Toggle save bar visibility
      if (sectionsWithSave.has(section)) {
        saveBar().classList.remove("hidden-save");
      } else {
        saveBar().classList.add("hidden-save");
      }
    });
  });
}

// ─── Slider fill ───
function updateSliderFill(slider: HTMLInputElement) {
  const min = Number(slider.min);
  const max = Number(slider.max);
  const val = Number(slider.value);
  const pct = ((val - min) / (max - min)) * 100;
  slider.style.background = `linear-gradient(to right, var(--blue) ${pct}%, var(--surface1) ${pct}%)`;
}

// ─── Config loading ───
async function loadConfig() {
  try {
    currentConfig = await invoke<PaveConfig>("get_config");
    gapSlider().value = String(currentConfig.gap_size);
    gapValue().textContent = `${currentConfig.gap_size}px`;
    const cr = currentConfig.corner_radius ?? 0;
    cornerSlider().value = String(cr);
    cornerValue().textContent = `${cr}px`;
    autostartToggle().checked = currentConfig.autostart;
    restoreSessionToggle().checked = currentConfig.restore_session;
    autoSurfaceToggle().checked = currentConfig.auto_surface_tabs;
    updateSliderFill(gapSlider());
    updateSliderFill(cornerSlider());
    takeSnapshot();
  } catch (e) {
    console.error("Failed to load config:", e);
  }
}

async function loadMonitors() {
  try {
    const monitors = await invoke<MonitorInfo[]>("get_monitors");
    const list = monitorsListEl();
    list.innerHTML = "";

    if (monitors.length === 0) {
      list.innerHTML = '<p class="loading">No monitors detected</p>';
      return;
    }

    for (const mon of monitors) {
      const row = document.createElement("label");
      row.className = "monitor-row";

      const checkbox = document.createElement("input");
      checkbox.type = "checkbox";
      checkbox.checked = !currentConfig.excluded_monitors.includes(mon.name);
      checkbox.dataset.monitorName = mon.name;

      const text = document.createElement("span");
      text.textContent = `${mon.name} (${mon.width}×${mon.height})`;

      row.appendChild(checkbox);
      row.appendChild(text);
      list.appendChild(row);
    }
  } catch (e) {
    console.error("Failed to load monitors:", e);
    monitorsListEl().innerHTML = '<p class="loading">Failed to load monitors</p>';
  }
}

interface MonitorInfo {
  name: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

// ─── Config collection ───
function collectConfig(): PaveConfig {
  const gap = parseInt(gapSlider().value, 10);
  const autostart = autostartToggle().checked;

  const excluded: string[] = [];
  const checkboxes = monitorsListEl().querySelectorAll<HTMLInputElement>(
    'input[type="checkbox"]'
  );
  for (const cb of checkboxes) {
    if (!cb.checked && cb.dataset.monitorName) {
      excluded.push(cb.dataset.monitorName);
    }
  }

  const corner = parseInt(cornerSlider().value, 10);
  const corner_radius = corner > 0 ? corner : null;

  const restore_session = restoreSessionToggle().checked;
  const auto_surface_tabs = autoSurfaceToggle().checked;

  return {
    gap_size: gap,
    excluded_monitors: excluded,
    autostart,
    corner_radius,
    presets: currentConfig.presets,
    restore_session,
    auto_surface_tabs,
  };
}

// ─── Dirty state tracking ───
function takeSnapshot() {
  const cfg = collectConfig();
  // Exclude presets from dirty tracking (managed separately)
  const { presets: _, ...rest } = cfg;
  configSnapshot = JSON.stringify(rest);
}

function getChanges(): string[] {
  const current = collectConfig();
  const { presets: _, ...currentRest } = current;
  const currentStr = JSON.stringify(currentRest);
  if (currentStr === configSnapshot) return [];

  const old = JSON.parse(configSnapshot);
  const changes: string[] = [];

  if (current.gap_size !== old.gap_size) {
    changes.push(`Gap size: ${old.gap_size}px → ${current.gap_size}px`);
  }
  if ((current.corner_radius ?? 0) !== (old.corner_radius ?? 0)) {
    changes.push(`Corner radius: ${old.corner_radius ?? 0}px → ${current.corner_radius ?? 0}px`);
  }
  if (current.autostart !== old.autostart) {
    changes.push(`Start on login: ${old.autostart ? "on" : "off"} → ${current.autostart ? "on" : "off"}`);
  }
  if (current.restore_session !== old.restore_session) {
    changes.push(`Restore session: ${old.restore_session ? "on" : "off"} → ${current.restore_session ? "on" : "off"}`);
  }
  if (current.auto_surface_tabs !== old.auto_surface_tabs) {
    changes.push(`Auto-surface: ${old.auto_surface_tabs ? "on" : "off"} → ${current.auto_surface_tabs ? "on" : "off"}`);
  }
  if (JSON.stringify(current.excluded_monitors.sort()) !== JSON.stringify(old.excluded_monitors.sort())) {
    changes.push("Monitor selection changed");
  }

  return changes;
}

// ─── Save ───
async function saveConfig() {
  const config = collectConfig();
  try {
    await invoke("update_config", { config });
    currentConfig = config;
    takeSnapshot();
    showToast("Settings saved", false);
  } catch (e) {
    showToast(`Failed to save: ${e}`, true);
  }
}

// ─── Toast system ───
function showToast(msg: string, isError: boolean) {
  const container = $("toast-container");
  const toast = document.createElement("div");
  toast.className = `toast${isError ? " toast-error" : ""}`;
  toast.textContent = msg;
  container.appendChild(toast);

  setTimeout(() => {
    toast.classList.add("toast-out");
    toast.addEventListener("animationend", () => toast.remove());
  }, 3000);
}

// ─── Presets ───
function renderPresets() {
  const list = presetsListEl();
  const presets = (currentConfig.presets || []).filter(
    (p) => p.name !== "__last_session__"
  );

  if (presets.length === 0) {
    list.innerHTML = '<p class="loading">No presets saved</p>';
    return;
  }

  list.innerHTML = "";
  for (const preset of presets) {
    const row = document.createElement("div");
    row.className = "preset-row";

    const name = document.createElement("span");
    name.className = "preset-name";
    name.textContent = preset.name;

    const info = document.createElement("span");
    info.className = "preset-info";
    info.textContent = `${preset.slots.length} window${preset.slots.length !== 1 ? "s" : ""}`;

    const actions = document.createElement("div");
    actions.className = "preset-actions";

    const activateBtn = document.createElement("button");
    activateBtn.className = "btn-small";
    activateBtn.textContent = "Activate";
    activateBtn.addEventListener("click", async () => {
      try {
        await invoke("activate_preset", { name: preset.name });
        showToast(`Activated: ${preset.name}`, false);
      } catch (e) {
        showToast(`Failed to activate: ${e}`, true);
      }
    });

    const deleteBtn = document.createElement("button");
    deleteBtn.className = "btn-small btn-danger";
    deleteBtn.textContent = "Delete";
    deleteBtn.addEventListener("click", async () => {
      try {
        await invoke("delete_preset", { name: preset.name });
        currentConfig.presets = currentConfig.presets.filter(
          (p) => p.name !== preset.name
        );
        renderPresets();
        showToast(`Deleted: ${preset.name}`, false);
      } catch (e) {
        showToast(`Failed to delete: ${e}`, true);
      }
    });

    actions.appendChild(activateBtn);
    actions.appendChild(deleteBtn);

    row.appendChild(name);
    row.appendChild(info);
    row.appendChild(actions);
    list.appendChild(row);
  }
}

function promptPresetName(): Promise<string | null> {
  return new Promise((resolve) => {
    const modal = $("preset-name-modal");
    const input = $("preset-name-input") as HTMLInputElement;
    input.value = "";
    modal.classList.remove("hidden");
    input.focus();

    function cleanup() {
      modal.classList.add("hidden");
      $("preset-name-ok").removeEventListener("click", onOk);
      $("preset-name-cancel").removeEventListener("click", onCancel);
      input.removeEventListener("keydown", onKey);
    }

    function onOk() {
      const val = input.value.trim();
      cleanup();
      resolve(val || null);
    }

    function onCancel() {
      cleanup();
      resolve(null);
    }

    function onKey(e: KeyboardEvent) {
      if (e.key === "Enter") onOk();
      if (e.key === "Escape") onCancel();
    }

    $("preset-name-ok").addEventListener("click", onOk);
    $("preset-name-cancel").addEventListener("click", onCancel);
    input.addEventListener("keydown", onKey);
  });
}

async function capturePreset() {
  const name = await promptPresetName();
  if (!name) return;

  try {
    const preset = await invoke<Preset>("capture_preset", { name });
    const idx = currentConfig.presets.findIndex((p) => p.name === preset.name);
    if (idx >= 0) {
      currentConfig.presets[idx] = preset;
    } else {
      currentConfig.presets.push(preset);
    }
    renderPresets();
    showToast(`Saved preset: ${preset.name} (${preset.slots.length} windows)`, false);
  } catch (e) {
    showToast(`Failed to capture: ${e}`, true);
  }
}

// ─── Unsaved changes modal ───
function showUnsavedModal(changes: string[]) {
  const modal = $("unsaved-modal");
  const changesList = $("modal-changes-list");
  changesList.innerHTML = "";
  for (const change of changes) {
    const li = document.createElement("li");
    li.textContent = change;
    changesList.appendChild(li);
  }
  modal.classList.remove("hidden");
}

function hideModal() {
  $("unsaved-modal").classList.add("hidden");
}

function setupCloseInterceptor() {
  getCurrentWindow().onCloseRequested(async (event) => {
    const changes = getChanges();
    if (changes.length === 0) return; // allow close

    event.preventDefault();
    showUnsavedModal(changes);
  });

  $("modal-save").addEventListener("click", async () => {
    hideModal();
    await saveConfig();
    getCurrentWindow().destroy();
  });

  $("modal-discard").addEventListener("click", () => {
    hideModal();
    getCurrentWindow().destroy();
  });

  $("modal-cancel").addEventListener("click", () => {
    hideModal();
  });
}

// ─── About links ───
function setupAboutLinks() {
  $("link-github").addEventListener("click", (e) => {
    e.preventDefault();
    openUrl("https://github.com/eaholum/pave");
  });
  $("link-kofi").addEventListener("click", (e) => {
    e.preventDefault();
    openUrl("https://ko-fi.com/eaholum");
  });
}

// ─── Init ───
window.addEventListener("DOMContentLoaded", async () => {
  setupNavigation();

  await loadConfig();
  await loadMonitors();
  renderPresets();

  // Slider events
  gapSlider().addEventListener("input", () => {
    gapValue().textContent = `${gapSlider().value}px`;
    updateSliderFill(gapSlider());
  });

  cornerSlider().addEventListener("input", () => {
    cornerValue().textContent = `${cornerSlider().value}px`;
    updateSliderFill(cornerSlider());
  });

  capturePresetBtn().addEventListener("click", capturePreset);
  saveBtn().addEventListener("click", saveConfig);

  setupCloseInterceptor();
  setupAboutLinks();
});
