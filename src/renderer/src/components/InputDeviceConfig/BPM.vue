<template>
  <grid
    v-searchTerms="{
      terms: ['bpm', 'tempo', 'beats per minute'],
      title: 'BPM Config',
      type: 'Panel',
    }"
    columns="4"
    class="device-config"
  >
    <c span="1..">BPM Source</c>
    <c span="3">
      <Select v-model="bpmSource" class="light">
        <option
          v-for="(source, index) in bpmSources"
          :key="index"
          :value="source"
        >
          {{ source }}
        </option>
      </Select>
    </c>
    <c span="1..">
      <grid columns="4">
        <c span="2">
          <Button class="light" :disabled="bpmSource !== 'tap'" @mousedown="tap"
            >TAP</Button
          >
        </c>
        <c span="2">{{ bpm }} BPM ({{ bpmSource }})</c>
      </grid>
    </c>
  </grid>
</template>

<script>
import Tt from "tap-tempo";

const tapTempo = new Tt();

export default {
  computed: {
    bpm() {
      return this.$modV.store.state.beats.bpm;
    },

    bpmSource: {
      get() {
        return this.$modV.store.state.beats.bpmSource;
      },

      set(source) {
        if (!this.bpmSources.includes(source)) {
          return;
        }

        this.$modV.store.commit("beats/SET_BPM_SOURCE", { source });
        this.$modV._store.commit("beats/SET_BPM_SOURCE", { source });
      },
    },

    bpmSources() {
      return this.$modV.store.state.beats.bpmSources;
    },
  },
  created() {
    tapTempo.on("tempo", (bpm) => {
      const tappedBpm = Math.round(bpm);

      if (this.bpm === tappedBpm) {
        return;
      }

      this.$modV.store.dispatch("beats/setBpm", {
        bpm: tappedBpm,
        source: "tap",
      });

      if (this.bpmSource === "tap") {
        this.$modV._store.commit("beats/SET_BPM", { bpm: tappedBpm });
      }
    });
  },

  methods: {
    toggleInput(id, value) {
      this.$modV.store.commit("midi/UPDATE_DEVICE", {
        id,
        key: "listenForInput",
        value,
      });
    },

    toggleClock(id, value) {
      this.$modV.store.commit("midi/UPDATE_DEVICE", {
        id,
        key: "listenForClock",
        value,
      });
    },

    tap() {
      tapTempo.tap();
    },
  },
};
</script>

<style scoped>
.device-config input,
.device-config textarea,
.device-config .select {
  max-width: 120px !important;
}

.device-config .range-control {
  max-width: 240px !important;
}
</style>
