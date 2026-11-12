<template>
  <grid
    v-infoView="{ title: iVTitle, body: iVBody, id: 'MIDI Config' }"
    v-searchTerms="{
      terms: ['midi', 'clock', 'input'],
      title: 'MIDI Input Config',
      type: 'Panel',
    }"
    columns="4"
    class="device-config"
  >
    <c v-if="engineMode" span="1..">
      <div class="device-config__notice" :title="engineUnsupportedTitle">
        {{ engineUnsupportedTitle }} — MIDI inputs cannot be listed or used.
      </div>
    </c>
    <c span="1">MIDI Inputs</c>
    <c span="1..">
      <grid columns="5">
        <c>Name</c>
        <c>Input</c>
        <c>Clock</c>
        <c>CC Latch</c>
        <c>NoteOn Latch</c>
      </grid>
    </c>
    <c v-for="(device, deviceId) in devices" :key="deviceId" span="1..">
      <grid columns="5">
        <c>{{ device.name }}</c>
        <c>
          <Button
            class="light"
            :disabled="engineMode"
            :title="engineMode ? engineUnsupportedTitle : null"
            @click="toggleInput(deviceId, !device.listenForInput)"
          >
            {{ device.listenForInput ? "On" : "Off" }}
          </Button>
        </c>
        <c>
          <Button
            class="light"
            :disabled="engineMode"
            :title="engineMode ? engineUnsupportedTitle : null"
            @click="toggleClock(deviceId, !device.listenForClock)"
          >
            {{ device.listenForClock ? "On" : "Off" }}
          </Button>
        </c>
        <c>
          <Button
            class="light"
            :disabled="engineMode"
            :title="engineMode ? engineUnsupportedTitle : null"
            @click="toggleCcLatch(deviceId, !device.ccLatch)"
          >
            {{ device.ccLatch ? "On" : "Off" }}
          </Button>
        </c>
        <c>
          <Button
            class="light"
            :disabled="engineMode"
            :title="engineMode ? engineUnsupportedTitle : null"
            @click="toggleNoteOnLatch(deviceId, !device.noteOnLatch)"
          >
            {{ device.noteOnLatch ? "On" : "Off" }}
          </Button>
        </c>
      </grid>
    </c>
  </grid>
</template>

<script>
import { ENGINE_UNSUPPORTED_TITLE } from "../../application/engine-unsupported";

export default {
  data() {
    return {
      iVTitle: "MIDI Input Config",
      iVBody:
        "Configure your MIDI inputs here. Use the toggles to accept MIDI inputs (CC/notes) and MIDI Clock per device.",
    };
  },

  computed: {
    engineMode() {
      return this.$modV?.engineMode === true;
    },

    engineUnsupportedTitle() {
      return ENGINE_UNSUPPORTED_TITLE;
    },

    devices() {
      return this.$modV.store.state.midi.devices;
    },
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

    toggleCcLatch(id, value) {
      this.$modV.store.commit("midi/UPDATE_DEVICE", {
        id,
        key: "ccLatch",
        value,
      });
    },

    toggleNoteOnLatch(id, value) {
      this.$modV.store.commit("midi/UPDATE_DEVICE", {
        id,
        key: "noteOnLatch",
        value,
      });
    },
  },
};
</script>

<style scoped>
.device-config__notice {
  font-style: italic;
  opacity: 0.8;
}
</style>
