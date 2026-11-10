// Downsamples the analyser's byte FFT data to a fixed number of segments.
//
// The analyser runs fftSize 2048 -> 1024 frequency bins, while the ISF
// audioFFT inputs, the worker's FFT aux canvas and Meyda all expect 512.
// The segments partition the source bins exactly once using floor() bounds,
// so every segment covers the same span of the spectrum (2 bins per segment
// for 1024 -> 512). Each segment keeps the loudest byte in its span: the
// byte values are a dB-scaled magnitude, so averaging pairs would halve a
// narrow peak rather than represent the segment fairly.
//
// Segment widths differ by at most one bin when the ratio is not exact,
// which is all "somewhat consistent segments" needs - the FFT is not an
// energy measurement and low-end resolution is not the goal.

const reduceByteFrequencyData = (data, targetLength) => {
  if (!data || data.length === 0 || !(targetLength > 0)) {
    return new Uint8Array(0);
  }

  const length = data.length;

  if (length <= targetLength) {
    const copy = new Uint8Array(length);
    copy.set(data);
    return copy;
  }

  const reduced = new Uint8Array(targetLength);
  let start = 0;

  for (let i = 0; i < targetLength; i += 1) {
    const end = Math.floor(((i + 1) * length) / targetLength);
    let max = 0;

    for (let j = start; j < end; j += 1) {
      if (data[j] > max) {
        max = data[j];
      }
    }

    reduced[i] = max;
    start = end;
  }

  return reduced;
};

export { reduceByteFrequencyData };
