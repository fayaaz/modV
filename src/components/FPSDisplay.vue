<template>
  <div>{{ Math.round($modV.store.state.metrics.fps) }} FPS</div>
</template>

<style scoped>
div {
  font-family: monospace;

  padding: 10px;
  color: #fff;
  background-color: rgba(0, 0, 0, 0.6);

  pointer-events: none;
}
</style>
