import BeatDetektor from "../../../../lib/BeatDetektor.js";

export default function () {
  const beatDetektor = new BeatDetektor(85, 169);
  const beatDetektorKick = new BeatDetektor.modules.vis.BassKick();

  this.updateBeatDetektor = (delta, features) => {
    if (!features || !features.complexSpectrum) {
      return;
    }

    // BeatDetektor tracks the gaps between triggers, so it needs an absolute
    // monotonically increasing timestamp in seconds, not the frame delta.
    const timer = performance.now() / 1000.0;

    beatDetektor.process(timer, features.complexSpectrum.real);
    beatDetektorKick.process(beatDetektor);
    const kick = beatDetektorKick.isKick();
    const bpm = beatDetektor.win_bpm_int_lo;

    this.store.commit("beats/SET_KICK", { kick });

    if (bpm && this.store.state.beats.bpm !== bpm) {
      this.store.dispatch("beats/setBpm", { bpm, source: "beatdetektor" });
    }
  };
}
