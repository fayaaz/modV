/* eslint-env worker */
import { v4 as uuidv4 } from "uuid";
import { createPreviewModule } from "./scene-preview-data";

// Hover previews run at a lower frame rate than the main output: they are
// small, transient and should not compete with the composition for GPU time.
const PREVIEW_FPS = 30;
const PREVIEW_FRAME_MS = 1000 / PREVIEW_FPS;

let preview = null;
const canvases = new Map();

function releaseModules(renderers) {
  if (!preview) {
    return;
  }

  const modules = preview.modules;
  preview.modules = [];

  for (let i = 0; i < modules.length; i++) {
    const module = modules[i];
    const renderer = renderers && renderers[module.meta.type];

    if (!renderer || !renderer.removeActiveModule) {
      continue;
    }

    try {
      renderer.removeActiveModule(module);
    } catch (error) {
      console.error("Could not dispose a Scene preview Module", error);
    }
  }
}

function clearCanvas(runtime = preview) {
  if (!runtime) {
    return;
  }

  const { canvas, context } = runtime;

  context.clearRect(0, 0, canvas.width, canvas.height);
}

function resizeCanvas(runtime, width, height) {
  const nextWidth = Math.max(1, Math.round(Number(width) || 320));
  const nextHeight = Math.max(1, Math.round(Number(height) || 180));

  if (runtime.canvas.width !== nextWidth) {
    runtime.canvas.width = nextWidth;
  }

  if (runtime.canvas.height !== nextHeight) {
    runtime.canvas.height = nextHeight;
  }
}

/**
 * Each grid cell owns a canvas that is transferred to the worker exactly once.
 * The worker keeps those canvases so it can switch the single live preview
 * between cells while every other cell keeps its last rendered frame.
 */
function ensureCanvas(key, canvas, width, height) {
  if (canvas) {
    canvases.set(key, canvas);
  }

  const resolvedCanvas = canvas || canvases.get(key);

  if (!resolvedCanvas) {
    return null;
  }

  if (preview && preview.canvas !== resolvedCanvas) {
    releaseModules(preview.renderers);
    preview = null;
  }

  if (!preview) {
    preview = {
      key,
      canvas: resolvedCanvas,
      context: resolvedCanvas.getContext("2d", { alpha: true }),
      modules: [],
      renderers: null,
      registered: null,
      sceneId: null,
      rowIndex: null,
      groupName: "",
      rendered: false,
      lastRenderAt: 0,
    };
  }

  preview.key = key;
  resizeCanvas(preview, width, height);

  return preview;
}

export function isScenePreviewActive() {
  return !!preview && preview.modules.length > 0;
}

/**
 * Stores a cell's transferred canvas for later hovers. The renderer can only
 * transfer a canvas once, so the worker has to keep it.
 */
export function attachScenePreviewCanvas(key, canvas, width, height) {
  return !!ensureCanvas(key, canvas, width, height);
}

export async function createScenePreview({
  key,
  sceneId,
  rowIndex,
  groupName,
  modules,
  registered,
  renderers,
  canvas,
  width,
  height,
}) {
  const runtime = ensureCanvas(key, canvas, width, height);

  if (!runtime) {
    return null;
  }

  releaseModules(runtime.renderers || renderers);
  runtime.renderers = renderers;
  runtime.registered = registered;
  runtime.sceneId = sceneId;
  runtime.rowIndex = Number(rowIndex);
  runtime.groupName = groupName || "";
  runtime.rendered = false;
  runtime.lastRenderAt = 0;
  clearCanvas();

  const moduleSnapshots = Array.isArray(modules) ? modules : [];

  for (let i = 0; i < moduleSnapshots.length; i++) {
    const snapshot = moduleSnapshots[i];
    const definition =
      registered &&
      (registered[snapshot.$moduleName] ||
        registered[snapshot.meta && snapshot.meta.name]);

    if (!definition) {
      continue;
    }

    const module = createPreviewModule(snapshot, definition, uuidv4());

    if (!module) {
      continue;
    }

    const renderer = renderers && renderers[module.meta.type];

    if (!renderer) {
      continue;
    }

    let added = false;

    try {
      if (renderer.addActiveModule) {
        await renderer.addActiveModule(module, definition, {
          canvas: runtime.canvas,
        });
      }

      added = true;

      if (definition.init) {
        const data = definition.init({
          canvas: runtime.canvas,
          data: { ...module.data },
          props: module.props,
        });

        if (data) {
          module.data = data;
        }
      }

      if (definition.resize && renderer.resizeModule) {
        const data = renderer.resizeModule({
          moduleDefinition: definition,
          canvas: runtime.canvas,
          data: { ...module.data },
          props: module.props,
          isGallery: false,
        });

        if (data) {
          module.data = data;
        }
      }

      runtime.modules.push(module);
    } catch (error) {
      console.error(`Could not prepare a Scene preview Module`, error);

      if (added && renderer.removeActiveModule) {
        try {
          renderer.removeActiveModule(module);
        } catch (removeError) {
          console.error(
            "Could not dispose a failed preview Module",
            removeError,
          );
        }
      }
    }
  }

  if (!runtime.modules.length) {
    clearScenePreview(renderers, key);
    return null;
  }

  return {
    key,
    sceneId,
    rowIndex: Number(rowIndex),
    groupName: groupName || "",
    moduleCount: runtime.modules.length,
  };
}

/**
 * Renders the preview Group into its private canvas. Returns `firstFrame` once
 * so the loop can publish readiness without committing every frame.
 */
export function renderScenePreview({
  registered,
  renderers,
  features,
  meyda,
  video,
  kick,
  fftCanvas,
  delta,
  osc,
}) {
  if (!preview || !preview.modules.length) {
    return null;
  }

  const now = performance.now();

  if (preview.lastRenderAt && now - preview.lastRenderAt < PREVIEW_FRAME_MS) {
    return null;
  }

  preview.lastRenderAt = now;

  const { canvas, context } = preview;
  const modules = preview.modules;

  clearCanvas();

  for (let i = 0; i < modules.length; i++) {
    const module = modules[i];
    const definition =
      registered[module.$moduleName] ||
      registered[module.meta && module.meta.name];
    const renderer = renderers[module.meta.type];

    if (
      !definition ||
      !renderer ||
      !module.meta.enabled ||
      !(module.meta.alpha > 0.001) ||
      (module.$status && module.$status.length)
    ) {
      continue;
    }

    context.save();
    context.globalAlpha = module.meta.alpha;
    context.globalCompositeOperation =
      module.meta.compositeOperation || "source-over";

    try {
      if (definition.update) {
        const data = renderer.updateModule({
          moduleDefinition: definition,
          props: module.props,
          data: { ...module.data },
          canvas,
          context,
          delta,
        });

        if (data) {
          module.data = data;
        }
      }

      renderer.render({
        canvas,
        context,
        delta,
        module: definition,
        moduleId: module.$id,
        features,
        meyda,
        video,
        props: module.props,
        data: module.data,
        pipeline: false,
        kick,
        fftCanvas,
        isGallery: true,
        osc,
      });
    } catch (error) {
      module.$status.push({
        type: "error",
        message: String(error),
      });
      console.error("Could not render a Scene preview Module", error);
    } finally {
      context.restore();
    }
  }

  const firstFrame = !preview.rendered;
  preview.rendered = true;

  return { firstFrame };
}

/**
 * Stops the live preview but leaves its last frame in the cell's canvas. The
 * canvas stays in the registry so hovering the same cell can resume.
 */
export function freezeScenePreview(renderers, key) {
  if (!preview || (key && preview.key !== key)) {
    return false;
  }

  releaseModules(renderers || preview.renderers);
  preview = null;

  return true;
}

/**
 * Removes the preview Modules and clears the live canvas. Freeze is used when
 * leaving a cell; this is for failed starts and explicit teardown.
 */
export function clearScenePreview(renderers, key) {
  if (!preview || (key && preview.key !== key)) {
    return;
  }

  releaseModules(renderers || preview.renderers);
  clearCanvas();
  preview = null;
}

/**
 * Drops any canvas and runtime for cells that no longer exist.
 */
export function pruneScenePreviews(keys) {
  const keep = new Set(Array.isArray(keys) ? keys : []);

  for (const key of canvases.keys()) {
    if (!keep.has(key)) {
      canvases.delete(key);
    }
  }

  if (preview && !keep.has(preview.key)) {
    releaseModules(preview.renderers);
    clearCanvas();
    preview = null;
    return true;
  }

  return false;
}

export function disposeScenePreview(renderers) {
  if (preview) {
    releaseModules(renderers || preview.renderers);
    clearCanvas();
    preview = null;
  }

  canvases.clear();
}
