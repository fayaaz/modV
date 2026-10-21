// Point10 scenes advance by elapsed time (`update(dt)`), but modV's 2d
// renderer passes the absolute requestAnimationFrame timestamp as `delta`
// (see renderers/2d/index.js). Treating that timestamp as a frame delta pins
// dt at the clamp and runs every scene several times too fast. Each adapter
// instance owns a clock instead, fed with performance.now() once per draw.
const MAX_FRAME_SECONDS = 0.1;

function createFrameClock() {
  let previous = null;

  return function tick(now) {
    if (previous === null) {
      previous = now;
      return 0;
    }

    const elapsed = (now - previous) / 1000;
    previous = now;

    return Math.min(MAX_FRAME_SECONDS, Math.max(0, elapsed));
  };
}

export { createFrameClock, MAX_FRAME_SECONDS };
