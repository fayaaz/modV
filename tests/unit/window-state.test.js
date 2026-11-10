import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";

import {
  createEmptyWindowState,
  nextOutputWindowKey,
  normalizeWindowState,
  readWindowState,
  resolveWindowBounds,
  saveWindowSnapshot,
  snapshotWindowState,
  writeWindowState,
} from "../../src/main/window-state.js";

const primaryDisplay = {
  id: 1,
  isPrimary: true,
  bounds: { x: 0, y: 0, width: 1920, height: 1080 },
  workArea: { x: 0, y: 0, width: 1920, height: 1040 },
};

const secondaryDisplay = {
  id: 2,
  isPrimary: false,
  bounds: { x: 1920, y: 0, width: 2560, height: 1440 },
  workArea: { x: 1920, y: 0, width: 2560, height: 1400 },
};

function tmpStatePath() {
  const directory = fs.mkdtempSync(
    path.join(os.tmpdir(), "vmodv-window-state-"),
  );
  return { directory, filePath: path.join(directory, "window-state.json") };
}

describe("window state persistence", () => {
  it("reads an empty state when the file is missing", () => {
    const { directory, filePath } = tmpStatePath();

    try {
      expect(readWindowState(filePath)).toEqual(createEmptyWindowState());
    } finally {
      fs.rmSync(directory, { recursive: true, force: true });
    }
  });

  it("reads an empty state when the file is corrupt", () => {
    const { directory, filePath } = tmpStatePath();

    try {
      fs.writeFileSync(filePath, "{not json");
      expect(readWindowState(filePath)).toEqual(createEmptyWindowState());
    } finally {
      fs.rmSync(directory, { recursive: true, force: true });
    }
  });

  it("round-trips window bounds, display and window mode", () => {
    const { directory, filePath } = tmpStatePath();
    const state = createEmptyWindowState();

    state.mainWindow = {
      x: 2000,
      y: 120,
      width: 1280,
      height: 720,
      displayId: 2,
      maximized: false,
      fullscreen: true,
    };
    state.windows["output-0"] = {
      x: 300,
      y: 40,
      width: 800,
      height: 600,
      displayId: 1,
      maximized: true,
      fullscreen: false,
    };

    try {
      expect(writeWindowState(filePath, state)).toBe(true);

      const loaded = readWindowState(filePath);

      expect(loaded.mainWindow).toEqual(state.mainWindow);
      expect(loaded.windows["output-0"]).toEqual(state.windows["output-0"]);
    } finally {
      fs.rmSync(directory, { recursive: true, force: true });
    }
  });

  it("creates the directory holding the state file", () => {
    const { directory } = tmpStatePath();
    const nested = path.join(directory, "nested", "window-state.json");

    try {
      expect(writeWindowState(nested, createEmptyWindowState())).toBe(true);
      expect(fs.existsSync(nested)).toBe(true);
    } finally {
      fs.rmSync(directory, { recursive: true, force: true });
    }
  });

  it("ignores state written by another version", () => {
    const state = normalizeWindowState({
      version: 99,
      mainWindow: { x: 10, y: 10, width: 800, height: 600 },
    });

    expect(state).toEqual(createEmptyWindowState());
  });

  it("drops entries with unusable bounds", () => {
    const state = normalizeWindowState({
      version: 1,
      mainWindow: { x: 10, y: 10, width: 0, height: 600 },
      windows: {
        "output-0": { x: 10, y: 10, width: 640, height: 480, displayId: 2 },
        "output-1": "nope",
      },
    });

    expect(state.mainWindow).toBeNull();
    expect(state.windows["output-0"]).toEqual({
      x: 10,
      y: 10,
      width: 640,
      height: 480,
      displayId: 2,
      maximized: false,
      fullscreen: false,
    });
    expect(state.windows["output-1"]).toBeUndefined();
  });
});

describe("resolveWindowBounds", () => {
  const fallback = { x: 0, y: 0, width: 1920, height: 1040 };

  it("uses the fallback when nothing has been saved", () => {
    const result = resolveWindowBounds(null, [primaryDisplay], fallback);

    expect(result).toEqual({ bounds: fallback, restored: false });
  });

  it("restores the exact bounds on the remembered display", () => {
    const saved = {
      x: 2100,
      y: 100,
      width: 1280,
      height: 720,
      displayId: 2,
      maximized: false,
      fullscreen: false,
    };

    const result = resolveWindowBounds(
      saved,
      [primaryDisplay, secondaryDisplay],
      fallback,
    );

    expect(result).toEqual({
      bounds: { x: 2100, y: 100, width: 1280, height: 720 },
      restored: true,
    });
  });

  it("restores the exact bounds when display ids changed but the bounds fit", () => {
    const saved = {
      x: 2100,
      y: 100,
      width: 1280,
      height: 720,
      displayId: 42,
    };
    const renamedDisplay = { ...secondaryDisplay, id: 7 };

    const result = resolveWindowBounds(
      saved,
      [primaryDisplay, renamedDisplay],
      fallback,
    );

    expect(result.bounds).toEqual({
      x: 2100,
      y: 100,
      width: 1280,
      height: 720,
    });
    expect(result.restored).toBe(true);
  });

  it("moves a window from a removed display back onto the primary display", () => {
    const saved = {
      x: 3000,
      y: 200,
      width: 1280,
      height: 720,
      displayId: 2,
    };

    const result = resolveWindowBounds(saved, [primaryDisplay], fallback);

    expect(result.restored).toBe(true);
    expect(result.bounds.width).toBe(1280);
    expect(result.bounds.height).toBe(720);
    expect(result.bounds.x).toBeGreaterThanOrEqual(primaryDisplay.workArea.x);
    expect(result.bounds.x + result.bounds.width).toBeLessThanOrEqual(
      primaryDisplay.workArea.x + primaryDisplay.workArea.width,
    );
    expect(result.bounds.y).toBeGreaterThanOrEqual(primaryDisplay.workArea.y);
    expect(result.bounds.y + result.bounds.height).toBeLessThanOrEqual(
      primaryDisplay.workArea.y + primaryDisplay.workArea.height,
    );
  });

  it("shrinks a remembered window when the display resolution got smaller", () => {
    const shrunkDisplay = {
      ...secondaryDisplay,
      bounds: { x: 1920, y: 0, width: 1600, height: 900 },
      workArea: { x: 1920, y: 0, width: 1600, height: 860 },
    };
    const saved = {
      x: 2000,
      y: 100,
      width: 2400,
      height: 1300,
      displayId: 2,
    };

    const result = resolveWindowBounds(
      saved,
      [primaryDisplay, shrunkDisplay],
      fallback,
    );

    expect(result.restored).toBe(true);
    expect(result.bounds).toEqual({
      x: 1920,
      y: 0,
      width: 1600,
      height: 860,
    });
  });

  it("pulls an off-screen window back onto a display", () => {
    const saved = {
      x: -5000,
      y: -4000,
      width: 800,
      height: 600,
      displayId: 1,
    };

    const result = resolveWindowBounds(saved, [primaryDisplay], fallback);

    expect(result.restored).toBe(true);
    expect(result.bounds).toEqual({ x: 0, y: 0, width: 800, height: 600 });
  });

  it("ignores unusable saved bounds", () => {
    const result = resolveWindowBounds(
      { x: 0, y: 0, width: 0, height: 0 },
      [primaryDisplay],
      fallback,
    );

    expect(result).toEqual({ bounds: fallback, restored: false });
  });

  it("falls back to the saved bounds when no displays are available", () => {
    const saved = { x: 20, y: 40, width: 640, height: 480, displayId: 1 };

    expect(resolveWindowBounds(saved, [], fallback)).toEqual({
      bounds: { x: 20, y: 40, width: 640, height: 480 },
      restored: true,
    });
  });
});

describe("saveWindowSnapshot", () => {
  const snapshot = {
    x: 10,
    y: 20,
    width: 800,
    height: 600,
    displayId: 1,
    maximized: false,
    fullscreen: false,
  };

  it("keeps the main window out of the output window slots", () => {
    const state = saveWindowSnapshot(
      createEmptyWindowState(),
      "mainWindow",
      snapshot,
    );

    expect(state.mainWindow).toEqual(snapshot);
    expect(state.windows).toEqual({});
  });

  it("stores output windows under their slot", () => {
    const state = saveWindowSnapshot(
      createEmptyWindowState(),
      "output-1",
      snapshot,
    );

    expect(state.mainWindow).toBeNull();
    expect(state.windows["output-1"]).toEqual(snapshot);
  });
});

describe("snapshotWindowState", () => {
  it("stores integer bounds, the display and window mode", () => {
    expect(
      snapshotWindowState({
        bounds: { x: 10.4, y: 20.6, width: 800.5, height: 600.4 },
        displayId: 3,
        maximized: true,
        fullscreen: false,
      }),
    ).toEqual({
      x: 10,
      y: 21,
      width: 801,
      height: 600,
      displayId: 3,
      maximized: true,
      fullscreen: false,
    });
  });

  it("returns null for unusable bounds", () => {
    expect(
      snapshotWindowState({ bounds: { x: 0, y: 0, width: NaN, height: 10 } }),
    ).toBeNull();
  });
});

describe("nextOutputWindowKey", () => {
  it("starts at zero", () => {
    expect(nextOutputWindowKey([])).toBe("output-0");
  });

  it("reuses the first free slot", () => {
    expect(nextOutputWindowKey(["output-0", "output-2"])).toBe("output-1");
  });

  it("allocates a new slot when the current ones are taken", () => {
    expect(nextOutputWindowKey(["output-0", "output-1"])).toBe("output-2");
  });
});
