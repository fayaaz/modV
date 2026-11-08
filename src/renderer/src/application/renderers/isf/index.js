import store from "../../worker/store";
import {
  resizeCanvas,
  resizeCanvases,
  clearCanvas,
  copyCanvas,
} from "../utils";
import { disposeISFRenderer, getModuleId } from "../disposal";

import {
  Renderer as ISFRenderer,
  Parser as ISFParser,
  Upgrader as ISFUpgrader,
} from "interactive-shader-format/src/main.js";
import { getFeatures } from "../../worker/audio-features";
import { getIsfGalleryCategory } from "../../module-category";
import { installFloatBufferSupport } from "./float-buffer-support";
import constants from "../../constants";

const isfCanvas = new OffscreenCanvas(256, 256);
// ISF only ever draws full-screen quads: multisampling costs fill rate and a
// resolve per pass without improving any edge.
const isfContext = isfCanvas.getContext("webgl2", {
  antialias: false,
  desynchronized: true,
  powerPreference: "high-performance",
  premultipliedAlpha: false,
});
installFloatBufferSupport(isfContext);
store.dispatch("outputs/addAuxillaryOutput", {
  name: "isf-buffer",
  context: isfContext,
  group: "buffer",
});

const isfCanvasGallery = new OffscreenCanvas(256, 256);
const isfContextGallery = isfCanvasGallery.getContext("webgl2", {
  antialias: false,
  desynchronized: true,
  powerPreference: "high-performance",
  premultipliedAlpha: false,
});
installFloatBufferSupport(isfContextGallery);
store.dispatch("outputs/addAuxillaryOutput", {
  name: "isf-buffer-gallery",
  context: isfContextGallery,
  group: "buffer",
});

const renderers = {};
const inputs = {};

function resize({ width, height }) {
  // Optimized: Use shared utility for consistent canvas resizing
  resizeCanvases([isfCanvas, isfCanvasGallery], width, height);
}

async function render({
  moduleId,
  canvas,
  context,
  pipeline,
  props,
  isGallery,
}) {
  const renderer = renderers[moduleId];
  const moduleInputs = inputs[moduleId];

  // Only update the audio data if the module has audio inputs to improve performance
  // for modules that don't use any audio inputs at all
  if (renderer.hasAudio) {
    const features = getFeatures();
    const byteFrequencyData = features.byteFrequencyData;
    const byteTimeDomainData = features.byteTimeDomainData;

    renderer.audio.setFrequencyValues(byteFrequencyData, byteFrequencyData);
    renderer.audio.setTimeDomainValues(byteTimeDomainData, byteTimeDomainData);
  }

  if (moduleInputs) {
    for (let i = 0, len = moduleInputs.length; i < len; i++) {
      const input = moduleInputs[i];

      if (input.TYPE === "image") {
        if (input.NAME in props) {
          const resolvedTexture =
            (props[input.NAME] && props[input.NAME].value) || canvas;
          renderer.setValue(input.NAME, resolvedTexture);
        } else {
          renderer.setValue(input.NAME, canvas);
        }
      } else if (input.TYPE === "event") {
        renderer.setValue(input.NAME, !!props[input.NAME]);
      } else {
        renderer.setValue(input.NAME, props[input.NAME]);
      }
    }
  }

  const resolvedIsfCanvas = isGallery ? isfCanvasGallery : isfCanvas;
  const resolvedIsfContext = isGallery ? isfContextGallery : isfContext;

  // Only resize the context this render targets. Resizing both contexts here
  // made every gallery/preview render shrink the live composition canvas to
  // thumbnail size and every live render grow it back, invalidating the live
  // drawing buffer on every preview frame.
  if (
    resolvedIsfCanvas.width !== canvas.width ||
    resolvedIsfCanvas.height !== canvas.height
  ) {
    resizeCanvas(resolvedIsfCanvas, canvas.width, canvas.height);
  }

  // Optimized: Use shared utility for consistent canvas clearing
  clearCanvas(resolvedIsfContext);
  renderer.draw(resolvedIsfCanvas);

  if (pipeline) {
    context.clearRect(0, 0, canvas.width, canvas.height);
  }

  // Optimized: Use shared utility for canvas copying
  copyCanvas(context, resolvedIsfCanvas);
}

/**
 * Called each frame to update the Module
 */
function updateModule({ module, props, data, canvas, context, delta }) {
  const { data: dataUpdated } = module.update({
    props,
    data,
    canvas,
    context,
    delta,
  });

  return dataUpdated ?? data;
}

function resizeModule({ moduleDefinition, canvas, data, props, isGallery }) {
  return moduleDefinition.resize({ canvas, data, props, isGallery });
}

function loadISF(
  { fragmentShader, vertexShader, isGallery = false },
  parseOnly = true,
) {
  const parser = new ISFParser();
  parser.parse(fragmentShader, vertexShader);
  if (parser.error) {
    throw new Error(`Parsing error: ${parser.error}`);
  }

  if (parser.isfVersion < 2) {
    fragmentShader = ISFUpgrader.convertFragment(fragmentShader);
    if (vertexShader) {
      vertexShader = ISFUpgrader.convertVertex(vertexShader);
    }
  }

  const resolvedIsfContext = isGallery ? isfContextGallery : isfContext;

  let renderer;
  if (!parseOnly) {
    renderer = new ISFRenderer(resolvedIsfContext, {
      useWebAudio: false,
      fftSize: constants.AUDIO_BUFFER_SIZE,
      hasAudio: parser.hasAudio,
    });
    renderer.loadSource(fragmentShader, vertexShader);

    if (!renderer.valid) {
      throw renderer.errorWithCorrectedLines;
    }
  }

  return { renderer, parser };
}

async function setupModule(moduleDefinition) {
  const { parser } = loadISF(moduleDefinition);

  moduleDefinition.meta.isfVersion = parser.isfVersion;
  moduleDefinition.meta.author = parser.metadata.CREDIT;
  moduleDefinition.meta.description = parser.metadata.DESCRIPTION;
  moduleDefinition.meta.version = parser.metadata.VSN;

  const galleryCategory = getIsfGalleryCategory(parser.metadata);
  if (galleryCategory) {
    moduleDefinition.meta.category = galleryCategory;
  }

  function addProp(name, prop) {
    if (!moduleDefinition.props) {
      moduleDefinition.props = {};
    }

    moduleDefinition.props[name] = prop;
  }

  const moduleInputs = parser.inputs;
  for (let i = 0, len = moduleInputs.length; i < len; i++) {
    const input = moduleInputs[i];

    switch (input.TYPE) {
      default:
        break;

      case "float":
        addProp(input.NAME, {
          type: "float",
          label: input.LABEL || input.NAME,
          default: typeof input.DEFAULT !== "undefined" ? input.DEFAULT : 0.0,
          min: input.MIN,
          max: input.MAX,
          step: 0.01,
        });
        break;

      case "bool":
        addProp(input.NAME, {
          type: "bool",
          label: input.LABEL || input.NAME,
          default: Boolean(input.DEFAULT),
        });
        break;

      case "long":
        addProp(input.NAME, {
          type: "enum",
          label: input.NAME,
          enum: input.VALUES.map((value, idx) => ({
            label: input.LABELS[idx],
            value,
            selected: value === input.DEFAULT,
          })),
        });
        break;

      case "color":
        addProp(input.NAME, {
          type: "vec4",
          label: input.LABEL || input.NAME,
          default: input.DEFAULT,
        });
        break;

      case "point2D":
        addProp(input.NAME, {
          type: "vec2",
          label: input.LABEL || input.NAME,
          default: input.DEFAULT || [0.0, 0.0],
          min: input.MIN,
          max: input.MAX,
        });
        break;

      case "image":
        moduleDefinition.meta.previewWithOutput = true;

        addProp(input.NAME, {
          type: "texture",
          label: input.LABEL || input.NAME,
        });

        break;

      case "event":
        addProp(input.NAME, {
          type: "event",
          label: input.LABEL || input.NAME,
        });
        break;
    }
  }

  moduleDefinition.draw = render;

  return moduleDefinition;
}

function addActiveModule({ $id: id, meta: { isGallery } }, moduleDefinition) {
  const { renderer, parser } = loadISF(
    { ...moduleDefinition, isGallery },
    false,
  );

  inputs[id] = parser.inputs;
  renderers[id] = renderer;
}

function removeActiveModule(moduleOrId) {
  const id = getModuleId(moduleOrId);

  if (!id) {
    return;
  }

  disposeISFRenderer(renderers[id]);
  delete renderers[id];
  delete inputs[id];
}

export default {
  setupModule,
  render,
  updateModule,
  resizeModule,
  resize,
  addActiveModule,
  removeActiveModule,
};
