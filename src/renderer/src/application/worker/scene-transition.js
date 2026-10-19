/* eslint-env worker */
import {
  Renderer as ISFRenderer,
  Parser as ISFParser,
  Upgrader as ISFUpgrader,
} from "interactive-shader-format/src/main.js";
import store from "./store";
import { GROUP_ENABLED } from "../constants";
import { NATIVE_TRANSITIONS } from "../scene-transition-defs";

const nativeTransitions = new Map(
  NATIVE_TRANSITIONS.map((transition) => [transition.name, transition]),
);
const renderers = new Map();

let canvas = null;
let gl = null;

const sceneCanvases = {
  from: null,
  to: null,
};

function resizeCanvas(target, width, height) {
  if (target.width !== width || target.height !== height) {
    target.width = width;
    target.height = height;
  }

  return target;
}

function ensureContext(width, height) {
  if (!canvas) {
    canvas = new OffscreenCanvas(width, height);
    gl = canvas.getContext("webgl2", {
      antialias: false,
      desynchronized: true,
      powerPreference: "high-performance",
      premultipliedAlpha: false,
    });
  } else {
    resizeCanvas(canvas, width, height);
  }

  return gl;
}

function getSceneCanvas(key, width, height) {
  let entry = sceneCanvases[key];

  if (!entry) {
    const sceneCanvas = new OffscreenCanvas(width, height);

    entry = {
      canvas: sceneCanvas,
      context: sceneCanvas.getContext("2d", { storage: "discardable" }),
    };

    sceneCanvases[key] = entry;
  }

  resizeCanvas(entry.canvas, width, height);
  return entry;
}

function composeGroups(entry, groups, width, height) {
  const { context } = entry;

  context.clearRect(0, 0, width, height);

  for (let i = 0, len = groups.length; i < len; i++) {
    const group = groups[i];

    if (
      !group ||
      group.hidden ||
      group.enabled !== GROUP_ENABLED ||
      !(group.alpha > 0) ||
      group.modules.length < 1
    ) {
      continue;
    }

    context.save();

    if (group.compositeOperation) {
      context.globalCompositeOperation = group.compositeOperation;
    }

    if (group.alpha < 1) {
      context.globalAlpha = group.alpha;
    }

    context.drawImage(group.context.context.canvas, 0, 0, width, height);
    context.restore();
  }

  return entry.canvas;
}

function findDefinition(name) {
  const native = nativeTransitions.get(name);

  if (native) {
    return native;
  }

  const registeredModules = store.state.modules.registered;

  for (const key of Object.keys(registeredModules)) {
    const module = registeredModules[key];

    if (
      module.meta.name === name &&
      module.props &&
      module.props.startImage &&
      module.props.endImage &&
      module.props.progress
    ) {
      return {
        name,
        fragmentShader: module.fragmentShader,
        vertexShader: module.vertexShader,
      };
    }
  }

  return null;
}

function getRenderer(definition) {
  if (renderers.has(definition.name)) {
    return renderers.get(definition.name);
  }

  let fragmentShader = definition.fragmentShader;
  let vertexShader = definition.vertexShader;
  const parser = new ISFParser();

  parser.parse(fragmentShader, vertexShader);

  if (parser.error) {
    console.error(
      `Could not parse transition "${definition.name}"`,
      parser.error,
    );
    return null;
  }

  if (parser.isfVersion < 2) {
    fragmentShader = ISFUpgrader.convertFragment(fragmentShader);

    if (vertexShader) {
      vertexShader = ISFUpgrader.convertVertex(vertexShader);
    }
  }

  const renderer = new ISFRenderer(gl, {
    useWebAudio: false,
    fftSize: 1024,
    hasAudio: false,
  });

  renderer.loadSource(fragmentShader, vertexShader);

  if (!renderer.valid) {
    console.error(
      `Could not compile transition "${definition.name}"`,
      renderer.errorWithCorrectedLines || renderer.error,
    );
    return null;
  }

  renderers.set(definition.name, renderer);
  return renderer;
}

/**
 * Composites the outgoing and incoming Scene Groups into two canvases and
 * runs the selected transition shader over them.
 *
 * @param {Object} args
 * @param {Array}  args.fromGroups Outgoing Scene Groups
 * @param {Array}  args.toGroups   Incoming Scene Groups
 * @param {String} args.name       Transition name
 * @param {Number} args.progress   0..1 transition progress
 * @param {Number} args.width
 * @param {Number} args.height
 * @returns {OffscreenCanvas|null}
 */
export function renderSceneTransition({
  fromGroups,
  toGroups,
  name,
  progress,
  width,
  height,
}) {
  if (!name) {
    return null;
  }

  const definition = findDefinition(name);

  if (!definition) {
    return null;
  }

  if (!ensureContext(width, height)) {
    return null;
  }

  const renderer = getRenderer(definition);

  if (!renderer) {
    return null;
  }

  const from = composeGroups(
    getSceneCanvas("from", width, height),
    fromGroups,
    width,
    height,
  );
  const to = composeGroups(
    getSceneCanvas("to", width, height),
    toGroups,
    width,
    height,
  );

  renderer.setValue("startImage", from);
  renderer.setValue("endImage", to);
  renderer.setValue("progress", Math.min(1, Math.max(0, progress)));

  gl.viewport(0, 0, width, height);
  gl.clearColor(0, 0, 0, 0);
  gl.clear(gl.COLOR_BUFFER_BIT);

  renderer.draw(canvas);

  return canvas;
}
