<template>
  <c v-if="hasLabelSlot" span="1.." class="label-row" :class="{ disabled }">
    <grid columns="4">
      <c span="1">
        <slot name="label" />
        <span v-if="hint" class="label-hint">{{ hint }}</span>
      </c>
      <c span="3">
        <Button @mousedown="open = !open">
          <img
            :class="{ flip: !open }"
            src="../assets/graphics/Arrow-vertical.svg?asset"
          />
        </Button>
      </c>
    </grid>
  </c>

  <slot v-if="!disabled && open" name="body" />
</template>

<script>
export default {
  props: {
    disabled: {
      type: Boolean,
      default: false,
    },
    // Optional one-line note next to the label (e.g. "not available with the
    // Rust engine yet"). Visible even when the row is disabled, where a
    // tooltip would not fire because the label row has `pointer-events: none`.
    hint: {
      type: String,
      default: "",
    },
  },

  data() {
    return {
      open: false,
    };
  },

  computed: {
    hasLabelSlot() {
      return !!this.$slots.label;
    },
  },
};
</script>

<style scoped>
.label-row {
  border-bottom: 1px solid #9a9a9a;
}

.label-row.disabled {
  opacity: 0.3;
  pointer-events: none;
}

.label-hint {
  margin-left: 0.5rem;
  font-size: 0.75em;
  font-weight: normal;
  color: var(--foreground-color-2, #9a9a9a);
}

.flip {
  transform: scaleY(-1);
}
</style>
