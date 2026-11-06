/* eslint-env worker */
import get from "lodash.get";
import store from "./store";
import constants, { GROUP_DISABLED, GROUP_ENABLED } from "../constants";
import NdiWorker from "./ndi-worker.worker.js?worker";
import { applyWindow } from "meyda/dist/esm/utilities";
import { renderSceneTransition } from "./scene-transition";
import { renderScenePreview } from "./scene-preview";
import { resolveInputLinkValue } from "./input-link-value";

const meyda = { windowing: applyWindow };

let bufferCanvas;
let bufferContext;
let lastScenesTickTime = null;
let scenesTickInFlight = false;
const fallbackByteFrequencyData = Array(constants.AUDIO_BUFFER_SIZE).fill(0);

export const ndiWorker = new NdiWorker();

async function loop(delta, features, fftOutput) {
  if (!bufferCanvas) {
    bufferCanvas = new OffscreenCanvas(300, 300);
    bufferContext = bufferCanvas.getContext("2d");
    store.dispatch("outputs/addAuxillaryOutput", {
      name: "loop-buffer",
      context: bufferContext,
      group: "buffer",
    });

    return;
  }

  const {
    beats: { kick },
    modules: { active, registered },
    groups: { groups },
    inputs: { inputs, inputLinks },
    outputs: {
      main,
      debug,
      debugContext,
      auxillary,
      webcam: video,
      webcams: webcamOutputs,
    },
    mediaStream,
    renderers,
    windows,
  } = store.state;

  const selectedVideoSources =
    (mediaStream && mediaStream.selectedVideoSources) || [];
  const orderedWebcams = selectedVideoSources
    .map((id) => webcamOutputs[id])
    .filter(Boolean);
  const webcams = orderedWebcams.length ? orderedWebcams : video ? [video] : [];

  const groupIndexRenderOrder = store.getters["groups/groupIndexRenderOrder"];

  if (!main) {
    return;
  }

  // Advance scene crossfades / performance mode phrase detection. The delta
  // passed to `loop` is the absolute requestAnimationFrame timestamp, so track
  // the real elapsed time between scene ticks here instead.
  //
  // Applying a Scene awaits many dispatches and can outlast a frame. The
  // render loop must not start a second tick (and therefore a second Scene
  // application) while one is still in flight, or the resulting Groups are
  // orphaned and rendered forever.
  if (!scenesTickInFlight) {
    const scenesTickTime = performance.now();
    const scenesDelta =
      lastScenesTickTime === null ? 0 : scenesTickTime - lastScenesTickTime;
    scenesTickInFlight = true;

    try {
      await store.dispatch("scenes/tick", { delta: scenesDelta });
    } catch (e) {
      console.error("scenes/tick threw an error", e);
    } finally {
      // Measure from when the tick finished: applying a Scene can take longer
      // than a frame, and that time must not be counted against the next
      // Scene's crossfade.
      lastScenesTickTime = performance.now();
      scenesTickInFlight = false;
    }
  }

  // Put fftBuffer on fftCanvas
  const byteFrequencyData =
    features.byteFrequencyData || fallbackByteFrequencyData;
  const fftBuffer = byteFrequencyData.reduce((arr, value) => {
    arr.push(value, value, value, 255);
    return arr;
  }, []);

  const uInt8Array = Uint8ClampedArray.from(fftBuffer);

  fftOutput.context.putImageData(
    new ImageData(uInt8Array, byteFrequencyData.length),
    0,
    0,
  );

  // Update Input Links
  const inputLinkKeys = Object.keys(inputLinks);
  const inputLinkKeysLength = inputLinkKeys.length;
  for (let i = 0; i < inputLinkKeysLength; ++i) {
    const inputId = inputLinkKeys[i];
    const bind = inputs[inputId];
    const link = inputLinks[inputId];

    const {
      type,
      location,
      data,
      data: { path },
    } = bind;

    const {
      type: linkType,
      location: linkLocation,
      args: linkArguments,
      min,
      max,
      source,
    } = link;

    const moduleId = store.state.inputs.inputs[inputId].data.moduleId;

    // check if the module is enabled for meyda sources, and if it is not, skip it
    // mutation linkTypes are also skipped as they're handled in index.worker.js in
    // the store commit subscription
    if (
      (moduleId &&
        (linkType === "mutation" ||
          (source === "meyda" &&
            moduleId &&
            !store.state.modules.active[moduleId].meta.enabled))) ||
      linkType === "mutation"
    ) {
      continue;
    }

    let rawValue;

    if (linkType === "getter" && linkArguments) {
      rawValue = store.getters[linkLocation](...linkArguments);
    } else if (linkType === "state") {
      rawValue = get(store.state, linkLocation);
    }

    // Audio features are undefined until the bridge delivers a frame (and
    // stay undefined with no input selected). Mapping that to a number would
    // produce NaN and permanently overwrite the linked prop, so leave the
    // input's saved value alone until a real value arrives.
    const value = resolveInputLinkValue(rawValue, min, max);

    if (value === undefined) {
      continue;
    }

    if (type === "action") {
      store.dispatch(location, { ...data, data: value, path });
    } else if (type === "commit") {
      store.commit(location, { ...data, data: value, path });
    }
  }

  const preProcessFrameFunctions =
    store.getters["plugins/preProcessFrame"] || [];
  const preProcessFrameFunctionsLength = preProcessFrameFunctions.length;

  for (let i = 0; i < preProcessFrameFunctionsLength; ++i) {
    preProcessFrameFunctions[i].preProcessFrame({
      features,
      store,
      props: preProcessFrameFunctions[i].$props,
    });
  }

  const renderersWithTick = store.getters["renderers/renderersWithTick"];
  const renderersWithTickLength = renderersWithTick.length;
  for (let i = 0; i < renderersWithTickLength; ++i) {
    renderersWithTick[i].tick();
  }

  // Hover previews render into their own small canvas. Only one is ever live,
  // and the preview module caps itself at 30fps so it stays off the main
  // composition's critical path.
  const previewResult = renderScenePreview({
    registered,
    renderers,
    features,
    meyda,
    video,
    kick,
    fftCanvas: fftOutput.context.canvas,
    delta,
    osc: store.state.osc.data,
  });

  if (previewResult && previewResult.firstFrame) {
    store.commit("scenePreview/SET_RENDERED");
  }

  let lastCanvas = main.canvas;
  const renderedCanvasByGroupId = {};

  const groupsLength = groupIndexRenderOrder.length;
  for (let i = 0; i < groupsLength; ++i) {
    const group = groups[groupIndexRenderOrder[i]];

    if (!group) {
      continue;
    }

    const isGalleryGroup = group.name === constants.GALLERY_GROUP_NAME;

    const groupModulesLength = group.modules.length;
    if (group.enabled === GROUP_DISABLED || group.alpha < 0.001) {
      continue;
    }

    const { clearing, inherit, inheritFrom, pipeline } = group;

    // A Group can lose its output (e.g. a Scene restore replaced the Group
    // list) and a Group can point at an auxiliary output that is gone. Skip
    // those Groups instead of throwing on every frame.
    const aux =
      group.drawToCanvasId && auxillary[group.drawToCanvasId]
        ? auxillary[group.drawToCanvasId].context
        : null;
    const groupContext = group.context ? group.context.context : null;

    if (!groupContext) {
      continue;
    }

    let drawTo = groupContext;

    if (aux) {
      drawTo = aux;
    }

    if (clearing) {
      drawTo.clearRect(0, 0, drawTo.canvas.width, drawTo.canvas.height);
    }

    if (pipeline && clearing && !isGalleryGroup) {
      bufferContext.clearRect(
        0,
        0,
        bufferContext.canvas.width,
        bufferContext.canvas.height,
      );
    }

    if (inherit) {
      const inheritedGroup =
        inheritFrom === -1
          ? null
          : groups.find((group) => group.id === inheritFrom);
      const canvasToInherit =
        inheritFrom === -1
          ? lastCanvas
          : inheritedGroup && inheritedGroup.context
            ? renderedCanvasByGroupId[inheritedGroup.id] ||
              inheritedGroup.context.context.canvas
            : null;

      if (canvasToInherit) {
        drawTo.drawImage(
          canvasToInherit,
          0,
          0,
          drawTo.canvas.width,
          drawTo.canvas.height,
        );

        if (pipeline && !isGalleryGroup) {
          bufferContext.drawImage(
            canvasToInherit,
            0,
            0,
            drawTo.canvas.width,
            drawTo.canvas.height,
          );
        }
      }
    }

    let firstModuleDrawn = false;
    for (let j = 0; j < groupModulesLength; ++j) {
      let canvas = drawTo.canvas;

      const module = active[group.modules[j]];

      if (
        !module ||
        !module.meta.enabled ||
        module.meta.alpha < 0.001 ||
        module.$status.length
      ) {
        continue;
      }

      if (pipeline && firstModuleDrawn && !isGalleryGroup) {
        canvas = bufferContext.canvas;
      } else if (pipeline && !isGalleryGroup) {
        canvas = drawTo.canvas;
        firstModuleDrawn = true;
      } else if (isGalleryGroup) {
        if (!aux) {
          continue;
        }

        canvas = aux.canvas;
      }

      drawTo.save();
      drawTo.globalAlpha = module.meta.alpha || 1;
      drawTo.globalCompositeOperation =
        module.meta.compositeOperation || "normal";

      const { props, data } = module;
      const moduleDefinition = registered[module.$moduleName];
      let moduleData = data;

      if (!moduleDefinition) {
        // The module was removed or unregistered while the Group still
        // referenced it. Skip it instead of throwing on every frame.
        continue;
      }

      if (moduleDefinition.update) {
        moduleData = renderers[module.meta.type].updateModule({
          moduleDefinition,
          props,
          data: { ...data },
          canvas,
          context: drawTo,
          delta,
        });

        store.commit("modules/UPDATE_ACTIVE_MODULE", {
          id: module.$id,
          key: "data",
          value: moduleData,
        });
      }

      if (pipeline && !isGalleryGroup) {
        drawTo.clearRect(0, 0, canvas.width, canvas.height);
        drawTo.drawImage(bufferCanvas, 0, 0, canvas.width, canvas.height);
      }

      renderers[module.meta.type].render({
        canvas,
        context: drawTo,
        delta,
        module: moduleDefinition,
        moduleId: module.$id,
        features,
        meyda,
        video: webcams[0] || video,
        videos: webcams,
        props,
        data: moduleData,
        pipeline,
        kick,
        fftCanvas: fftOutput.context.canvas,
        isGallery: isGalleryGroup,
        osc: store.state.osc.data,
      });
      drawTo.restore();

      if (pipeline && !isGalleryGroup) {
        bufferContext.clearRect(0, 0, canvas.width, canvas.height);
        bufferContext.drawImage(
          drawTo.canvas,
          0,
          0,
          canvas.width,
          canvas.height,
        );

        drawTo.clearRect(0, 0, canvas.width, canvas.height);
        drawTo.drawImage(
          bufferCanvas,
          0,
          0,
          bufferCanvas.width,
          bufferCanvas.height,
        );
      }
    }

    const effectiveCanvas = drawTo.canvas;

    renderedCanvasByGroupId[group.id] = effectiveCanvas;

    // A fading ghost renders its own outgoing visuals, but must not become the
    // canvas the next row inherits from: the incoming Group owns the row.
    if (
      !group.hidden &&
      group.enabled === GROUP_ENABLED &&
      group.transitionGhost !== true
    ) {
      lastCanvas = effectiveCanvas;
    }
  }

  const windowKeys = Object.keys(windows);
  const windowsLength = windowKeys.length;

  main.clearRect(0, 0, main.canvas.width, main.canvas.height);

  const scenesState = store.state.scenes;
  const transitionName = store.getters["scenes/currentTransitionName"];
  const shaderTransition =
    scenesState.transitioning && transitionName && transitionName !== "Fade";

  let transitionCanvas = null;

  if (shaderTransition) {
    const fromGroups = [];
    const toGroups = [];
    const transitionGroupIds = store.getters["scenes/transitionGroupIds"];
    const fromIds = new Set(transitionGroupIds ? transitionGroupIds.from : []);
    const toIds = new Set(transitionGroupIds ? transitionGroupIds.to : []);

    for (let i = 0; i < groupsLength; ++i) {
      const group = groups[i];

      if (group.name === constants.GALLERY_GROUP_NAME) {
        continue;
      }

      if (toIds.has(group.id)) {
        toGroups.push(group);
      } else if (fromIds.has(group.id)) {
        fromGroups.push(group);
      }
    }

    const transitionResult = renderSceneTransition({
      fromGroups,
      toGroups,
      name: transitionName,
      progress: store.getters["scenes/transitionProgress"],
      width: main.canvas.width,
      height: main.canvas.height,
    });

    if (transitionResult) {
      transitionCanvas = transitionResult.canvas;

      // The shader could not be created/compiled. Complete the in-flight
      // Scene change with the regular group crossfade instead.
      if (transitionResult.fallback) {
        await store.dispatch("scenes/fallbackToFade");
      }
    } else {
      await store.dispatch("scenes/fallbackToFade");
    }
  }

  if (transitionCanvas) {
    main.drawImage(
      transitionCanvas,
      0,
      0,
      main.canvas.width,
      main.canvas.height,
    );
  } else {
    // Skip the Gallery Group by identity rather than assuming it is always the
    // last entry: a layout without the Gallery panel has no Gallery Group, and
    // blindly dropping the last render-order entry would silently skip the
    // final performance row (usually Effects).
    for (let i = 0; i < groupsLength; ++i) {
      const group = groups[groupIndexRenderOrder[i]];

      if (!group || group.name === constants.GALLERY_GROUP_NAME) {
        continue;
      }

      const {
        compositeOperation,
        context: { context },
        alpha,
        enabled,
        modules,
      } = group;
      const groupModulesLength = modules.length;
      if (enabled !== GROUP_ENABLED || groupModulesLength < 1 || !(alpha > 0)) {
        continue;
      }

      main.save();
      if (compositeOperation) {
        main.globalCompositeOperation = compositeOperation;
      }

      if (alpha < 1) {
        main.globalAlpha = alpha;
      }

      main.drawImage(context.canvas, 0, 0);
      main.restore();
    }
  }

  for (let i = 0; i < windowsLength; ++i) {
    const { outputId } = windows[windowKeys[i]];
    const outputContext = store.state.outputs.auxillary[outputId];

    if (outputContext) {
      const { width, height } = outputContext.context.canvas;
      outputContext.context.clearRect(0, 0, width, height);
      outputContext.context.drawImage(main.canvas, 0, 0, width, height);
    }
  }

  if (main.canvas.width !== bufferCanvas.width) {
    bufferCanvas.width = main.canvas.width;
  }

  if (main.canvas.height !== bufferCanvas.height) {
    bufferCanvas.height = main.canvas.height;
  }

  if (debug && debugContext) {
    const { canvas: debugCanvas } = debugContext;
    const canvasToDebug = store.getters["outputs/canvasToDebug"];

    if (canvasToDebug) {
      const { width: dWidth, height: dHeight } = canvasToDebug.context.canvas;
      const aspectRatio = dWidth / dHeight;
      debugCanvas.height = debugCanvas.width / aspectRatio;

      debugContext.clearRect(0, 0, debugCanvas.width, debugCanvas.height);
      debugContext.drawImage(
        canvasToDebug.context.canvas,
        0,
        0,
        debugCanvas.width,
        debugCanvas.height,
      );
    }
  }

  const postProcessFrameFunctions =
    store.getters["plugins/postProcessFrame"] || [];
  const postProcessFrameFunctionsLength = postProcessFrameFunctions.length;

  for (let i = 0; i < postProcessFrameFunctionsLength; ++i) {
    postProcessFrameFunctions[i].postProcessFrame({
      canvas: main.canvas,
      features,
      store,
      props: postProcessFrameFunctions[i].$props,
    });
  }

  // Gotta transfer after everything else as it transfers ownership of the canvas content
  // i.e. main.canvas goes blank

  if (store.state.ndi.outputEnabled) {
    const imageBitmap = await main.canvas.transferToImageBitmap();
    ndiWorker.postMessage(
      {
        type: "imageBitmap",
        payload: { imageBitmap },
      },
      [imageBitmap],
    );
  }
}

export default loop;
