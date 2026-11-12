/**
 * Point10/scene frame trail diagnostics.
 *
 * Writes a once-per-second snapshot to the daily app log describing what the
 * render loop composed: the main canvas as it was at the start of the loop
 * (i.e. the frame the first `inheritFrom: -1` Group would copy), the canvas
 * each Group drew into, the canvas a Group inherited, and the final main
 * canvas. A growing "lit" fraction across frames on a canvas that the live
 * scene never fully paints is a frame-feedback loop.
 */

const state = {
  scratch: null,
  nextAt: 0,
};

function getScratch(width = 64, height = 36) {
  if (
    !state.scratch ||
    state.scratch.width !== width ||
    state.scratch.height !== height
  ) {
    state.scratch = new OffscreenCanvas(width, height);
  }

  return state.scratch;
}

export function sampleCanvas(canvas) {
  if (!canvas || !canvas.width || !canvas.height) {
    return null;
  }

  try {
    const width = 64;
    const height = 36;
    const scratch = getScratch(width, height);
    const context = scratch.getContext("2d");

    context.clearRect(0, 0, width, height);
    context.drawImage(canvas, 0, 0, width, height);

    const data = context.getImageData(0, 0, width, height).data;
    let lit = 0;
    let rgb = 0;
    let alpha = 0;

    for (let i = 0; i < data.length; i += 4) {
      if (data[i + 3] > 8) {
        lit += 1;
      }

      rgb += data[i] + data[i + 1] + data[i + 2];
      alpha += data[i + 3];
    }

    const pixels = width * height;

    return {
      lit: Number((lit / pixels).toFixed(3)),
      rgb: Math.round(rgb / (pixels * 3)),
      alpha: Math.round(alpha / pixels),
    };
  } catch (error) {
    return null;
  }
}

function formatSample(sample) {
  if (!sample) {
    return "none";
  }

  return `lit:${sample.lit} rgb:${sample.rgb} alpha:${sample.alpha}`;
}

export function diagShouldLog(now) {
  if (now < state.nextAt) {
    return false;
  }

  state.nextAt = now + 1000;
  return true;
}

export function writeDiagLine(line, tag = "render-diag") {
  try {
    if (typeof self === "undefined" || !self.__userData) {
      return;
    }

    const fs = require("fs");
    const path = require("path");
    const now = new Date();
    const stamp = [
      now.getFullYear(),
      String(now.getMonth() + 1).padStart(2, "0"),
      String(now.getDate()).padStart(2, "0"),
    ].join("-");
    const directory = path.join(self.__userData, "logs");

    fs.mkdirSync(directory, { recursive: true });
    fs.appendFileSync(
      path.join(directory, `vmodv-${stamp}.log`),
      `${now.toISOString()} INFO [${tag}] ${line}\n`,
    );
  } catch (error) {
    // Diagnostics must never break rendering.
  }
}

export function formatGroupDiag({
  index,
  group,
  drawTo,
  inheritSource,
  inheritSourceName,
  groupSample,
  moduleNames,
}) {
  return (
    `group idx=${index} name="${group.name}" id=${group.id} ` +
    `enabled=${group.enabled} hidden=${group.hidden} alpha=${group.alpha} ` +
    `clearing=${group.clearing} inherit=${group.inherit} ` +
    `inheritFrom=${group.inheritFrom} pipeline=${group.pipeline} ` +
    `canvas=${drawTo.canvas.width}x${drawTo.canvas.height} ` +
    `after=${formatSample(groupSample)} ` +
    `inheritSrc=${inheritSourceName}:${formatSample(inheritSource)} ` +
    `modules=${moduleNames || "none"}`
  );
}

export function formatFrameDiag({
  mainPrev,
  mainAfterClear,
  mainFinal,
  windowCount,
  ndi,
}) {
  return (
    `frame mainPrev=${formatSample(mainPrev)} ` +
    `mainAfterClear=${formatSample(mainAfterClear)} ` +
    `mainFinal=${formatSample(mainFinal)} ` +
    `windows=${windowCount} ndi=${ndi}`
  );
}
