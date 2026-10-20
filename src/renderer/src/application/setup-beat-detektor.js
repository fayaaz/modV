import BeatDetektor from "../../../../lib/BeatDetektor.js";
import { TempoDetector } from "./tempo-detector";

const STORE_UPDATE_INTERVAL = 1;
const ONSET_SPECTRUM_PORTION = 0.25;

function createOnsetStrength() {
  let previousMagnitudes = null;

  return (features) => {
    const spectrum = features && features.complexSpectrum;

    if (spectrum && spectrum.real) {
      const real = spectrum.real;
      const imaginary = spectrum.imag || [];
      const binCount = Math.max(
        8,
        Math.floor(real.length * ONSET_SPECTRUM_PORTION),
      );

      if (!previousMagnitudes || previousMagnitudes.length !== binCount) {
        previousMagnitudes = new Float64Array(binCount);

        for (let i = 0; i < binCount; i++) {
          previousMagnitudes[i] = Math.log1p(
            Math.hypot(real[i] || 0, imaginary[i] || 0),
          );
        }

        return 0;
      }

      let flux = 0;

      for (let i = 0; i < binCount; i++) {
        const magnitude = Math.log1p(
          Math.hypot(real[i] || 0, imaginary[i] || 0),
        );
        const rise = magnitude - previousMagnitudes[i];

        if (rise > 0) {
          flux += rise;
        }

        previousMagnitudes[i] = magnitude;
      }

      return flux / binCount;
    }

    const byteFrequencyData = features && features.byteFrequencyData;

    if (byteFrequencyData) {
      if (
        !previousMagnitudes ||
        previousMagnitudes.length !== byteFrequencyData.length
      ) {
        previousMagnitudes = Float64Array.from(byteFrequencyData);
        return 0;
      }

      let flux = 0;

      for (let i = 0; i < byteFrequencyData.length; i++) {
        const rise = byteFrequencyData[i] - previousMagnitudes[i];

        if (rise > 0) {
          flux += rise;
        }

        previousMagnitudes[i] = byteFrequencyData[i];
      }

      return flux / byteFrequencyData.length;
    }

    return 0;
  };
}

export default function () {
  const beatDetektor = new BeatDetektor(85, 169);
  const beatDetektorKick = new BeatDetektor.modules.vis.BassKick();
  const tempoDetector = new TempoDetector({
    minBpm: 70,
    maxBpm: 200,
    confidenceThreshold: 0.35,
  });
  const getOnsetStrength = createOnsetStrength();
  let lastStoreUpdate = -Infinity;

  this.updateBeatDetektor = (delta, features) => {
    if (
      !features ||
      (!features.complexSpectrum && !features.byteFrequencyData)
    ) {
      return;
    }

    // BeatDetektor tracks the gaps between triggers, so it needs an absolute
    // monotonically increasing timestamp in seconds, not the frame delta.
    const timer = performance.now() / 1000.0;

    if (features.complexSpectrum && features.complexSpectrum.real) {
      beatDetektor.process(timer, features.complexSpectrum.real);
      beatDetektorKick.process(beatDetektor);

      const kick = beatDetektorKick.isKick();
      this.store.commit("beats/SET_KICK", { kick });
    }

    tempoDetector.process(timer, getOnsetStrength(features));

    if (timer - lastStoreUpdate < STORE_UPDATE_INTERVAL) {
      return;
    }

    lastStoreUpdate = timer;

    const bpm = Math.round(tempoDetector.bpm);
    const confident =
      tempoDetector.confidence >= tempoDetector.confidenceThreshold;

    if (
      confident &&
      bpm >= tempoDetector.minBpm &&
      bpm <= tempoDetector.maxBpm &&
      this.store.state.beats.bpm !== bpm
    ) {
      this.store.dispatch("beats/setBpm", { bpm, source: "beatdetektor" });
    }
  };
}
