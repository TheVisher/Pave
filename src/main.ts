import { invoke } from "@tauri-apps/api/core";

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
}

interface MonitorInfo {
  name: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

let currentConfig: PaveConfig = {
  gap_size: 15,
  excluded_monitors: [],
  autostart: false,
  corner_radius: null,
  presets: [],
};

const gapSlider = () => document.getElementById("gap-slider") as HTMLInputElement;
const gapValue = () => document.getElementById("gap-value")!;
const cornerSlider = () => document.getElementById("corner-slider") as HTMLInputElement;
const cornerValue = () => document.getElementById("corner-value")!;
const monitorsListEl = () => document.getElementById("monitors-list")!;
const autostartToggle = () => document.getElementById("autostart-toggle") as HTMLInputElement;
const presetsListEl = () => document.getElementById("presets-list")!;
const capturePresetBtn = () => document.getElementById("capture-preset-btn")!;
const saveBtn = () => document.getElementById("save-btn")!;
const statusMsg = () => document.getElementById("status-msg")!;

async function loadConfig() {
  try {
    currentConfig = await invoke<PaveConfig>("get_config");
    gapSlider().value = String(currentConfig.gap_size);
    gapValue().textContent = String(currentConfig.gap_size);
    const cr = currentConfig.corner_radius ?? 0;
    cornerSlider().value = String(cr);
    cornerValue().textContent = String(cr);
    autostartToggle().checked = currentConfig.autostart;
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
      text.textContent = `${mon.name} (${mon.width}x${mon.height})`;

      row.appendChild(checkbox);
      row.appendChild(text);
      list.appendChild(row);
    }
  } catch (e) {
    console.error("Failed to load monitors:", e);
    monitorsListEl().innerHTML = '<p class="loading">Failed to load monitors</p>';
  }
}

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

  return { gap_size: gap, excluded_monitors: excluded, autostart, corner_radius, presets: currentConfig.presets };
}

async function saveConfig() {
  const config = collectConfig();
  try {
    await invoke("update_config", { config });
    currentConfig = config;
    showStatus("Settings saved", false);
  } catch (e) {
    showStatus(`Failed to save: ${e}`, true);
  }
}

function showStatus(msg: string, isError: boolean) {
  const el = statusMsg();
  el.textContent = msg;
  el.className = `status-msg ${isError ? "error" : "success"}`;
  setTimeout(() => {
    el.textContent = "";
    el.className = "status-msg";
  }, 3000);
}

function renderPresets() {
  const list = presetsListEl();
  const presets = currentConfig.presets || [];

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
        showStatus(`Activated: ${preset.name}`, false);
      } catch (e) {
        showStatus(`Failed to activate: ${e}`, true);
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
        showStatus(`Deleted: ${preset.name}`, false);
      } catch (e) {
        showStatus(`Failed to delete: ${e}`, true);
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

async function capturePreset() {
  const name = prompt("Preset name:");
  if (!name || !name.trim()) return;

  try {
    const preset = await invoke<Preset>("capture_preset", { name: name.trim() });
    // Update local config
    const idx = currentConfig.presets.findIndex((p) => p.name === preset.name);
    if (idx >= 0) {
      currentConfig.presets[idx] = preset;
    } else {
      currentConfig.presets.push(preset);
    }
    renderPresets();
    showStatus(`Saved preset: ${preset.name} (${preset.slots.length} windows)`, false);
  } catch (e) {
    showStatus(`Failed to capture: ${e}`, true);
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  // Load config first, then monitors and presets (display depends on config)
  await loadConfig();
  await loadMonitors();
  renderPresets();

  gapSlider().addEventListener("input", () => {
    gapValue().textContent = gapSlider().value;
  });

  cornerSlider().addEventListener("input", () => {
    cornerValue().textContent = cornerSlider().value;
  });

  capturePresetBtn().addEventListener("click", capturePreset);
  saveBtn().addEventListener("click", saveConfig);
});
