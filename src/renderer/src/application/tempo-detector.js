/**
 * Dependency-free tempo tracker.
 *
 * Onset strengths are accumulated into a uniformly sampled envelope, then
 * scored with a harmonic autocorrelation/comb filter across the supported BPM
 * range. A midpoint-energy check corrects octave errors (for example, 174 BPM
 * material whose every other beat is slightly stronger). Estimates are
 * median-smoothed and confidence is smoothed over time so the reported tempo
 * does not jump between analyses.
 */

export const MIN_BPM = 70;
export const MAX_BPM = 200;

const DEFAULT_OPTIONS = {
  minBpm: MIN_BPM,
  maxBpm: MAX_BPM,
  historySeconds: 10,
  analysisRate: 100,
  analysisInterval: 0.2,
  minHistorySeconds: 3,
  smoothingWindow: 5,
  confidenceSmoothing: 0.4,
  octaveMidpointRatio: 0.35,
  confidenceThreshold: 0.25,
};

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function median(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);

  if (sorted.length % 2 === 1) {
    return sorted[middle];
  }

  return (sorted[middle - 1] + sorted[middle]) / 2;
}

export class TempoDetector {
  constructor(options = {}) {
    const config = { ...DEFAULT_OPTIONS, ...options };

    this.minBpm = clamp(config.minBpm, 1, 400);
    this.maxBpm = clamp(config.maxBpm, this.minBpm + 1, 400);
    this.historySeconds = config.historySeconds;
    this.analysisRate = config.analysisRate;
    this.analysisInterval = config.analysisInterval;
    this.minHistorySeconds = config.minHistorySeconds;
    this.smoothingWindow = Math.max(1, config.smoothingWindow);
    this.confidenceSmoothing = clamp(config.confidenceSmoothing, 0, 1);
    this.octaveMidpointRatio = config.octaveMidpointRatio;
    this.confidenceThreshold = config.confidenceThreshold;

    this.frameDuration = 1 / this.analysisRate;
    this.historyFrames = Math.max(
      2,
      Math.round(this.historySeconds * this.analysisRate),
    );

    this._samples = [];
    this._estimates = [];
    this._lastAnalysisTime = -Infinity;

    this.bpm = 0;
    this.confidence = 0;
  }

  /**
   * Feed one sample of the onset-strength envelope.
   *
   * @param {number} timeSeconds Monotonic timestamp in seconds.
   * @param {number} onsetStrength Non-negative onset strength.
   */
  process(timeSeconds, onsetStrength) {
    if (!Number.isFinite(timeSeconds)) {
      return;
    }

    const strength = Number.isFinite(onsetStrength)
      ? Math.max(0, onsetStrength)
      : 0;

    const lastSample = this._samples[this._samples.length - 1];

    if (lastSample && timeSeconds < lastSample.time) {
      this.reset();
    }

    this._samples.push({ time: timeSeconds, value: strength });

    const cutoff = timeSeconds - this.historySeconds - 1;
    let removeCount = 0;

    while (
      removeCount < this._samples.length &&
      this._samples[removeCount].time < cutoff
    ) {
      removeCount += 1;
    }

    if (removeCount > 0) {
      this._samples.splice(0, removeCount);
    }

    if (timeSeconds - this._lastAnalysisTime >= this.analysisInterval) {
      this._lastAnalysisTime = timeSeconds;
      this._analyze();
    }
  }

  /**
   * Alias kept for callers that think in discrete onsets.
   */
  addOnset(timeSeconds, onsetStrength = 1) {
    this.process(timeSeconds, onsetStrength);
  }

  reset() {
    this._samples.length = 0;
    this._estimates.length = 0;
    this._lastAnalysisTime = -Infinity;
    this.bpm = 0;
    this.confidence = 0;
  }

  _buildEnvelope() {
    const frames = this.historyFrames;
    const envelope = new Float64Array(frames);

    if (this._samples.length < 2) {
      return envelope;
    }

    const newest = this._samples[this._samples.length - 1].time;
    const start = newest - this.historySeconds;
    const lastIndex = this._samples.length - 1;
    let sampleIndex = 0;

    for (let i = 0; i < frames; i++) {
      const time = start + i * this.frameDuration;

      while (
        sampleIndex < lastIndex &&
        this._samples[sampleIndex + 1].time <= time
      ) {
        sampleIndex += 1;
      }

      if (sampleIndex >= lastIndex) {
        envelope[i] = this._samples[lastIndex].value;
        continue;
      }

      const previous = this._samples[sampleIndex];
      const next = this._samples[sampleIndex + 1];
      const span = next.time - previous.time;

      if (span > 0.08) {
        // Sparse event-style input: keep isolated onsets as short pulses
        // instead of interpolating a constant value between them.
        const toPrevious = Math.max(0, time - previous.time);
        const toNext = Math.max(0, next.time - time);
        const distance = Math.min(toPrevious, toNext);
        const nearestValue = toPrevious <= toNext ? previous.value : next.value;

        envelope[i] =
          distance < 0.05 ? nearestValue * (1 - distance / 0.05) : 0;
        continue;
      }

      const fraction = span > 0 ? (time - previous.time) / span : 0;
      const clampedFraction = clamp(fraction, 0, 1);

      envelope[i] =
        previous.value + (next.value - previous.value) * clampedFraction;
    }

    let peak = 0;

    for (let i = 0; i < frames; i++) {
      if (envelope[i] > peak) {
        peak = envelope[i];
      }
    }

    if (peak <= 0) {
      return envelope;
    }

    let mean = 0;

    for (let i = 0; i < frames; i++) {
      envelope[i] /= peak;
      mean += envelope[i];
    }

    mean /= frames;
    peak = 0;

    for (let i = 0; i < frames; i++) {
      envelope[i] = Math.max(0, envelope[i] - mean);

      if (envelope[i] > peak) {
        peak = envelope[i];
      }
    }

    if (peak > 0) {
      for (let i = 0; i < frames; i++) {
        envelope[i] /= peak;
      }
    }

    return envelope;
  }

  _autocorrelation(envelope, lag) {
    const frames = envelope.length;

    if (lag <= 0 || lag >= frames - 1) {
      return 0;
    }

    const baseLag = Math.floor(lag);
    const fraction = lag - baseLag;
    const last = frames - baseLag - 2;

    let mean = 0;

    for (let i = 0; i < frames; i++) {
      mean += envelope[i];
    }

    mean /= frames;

    let correlation = 0;
    let energyA = 0;
    let energyB = 0;

    for (let i = 0; i < last; i++) {
      const a = envelope[i] - mean;
      const b =
        (envelope[i + baseLag] - mean) * (1 - fraction) +
        (envelope[i + baseLag + 1] - mean) * fraction;

      correlation += a * b;
      energyA += a * a;
      energyB += b * b;
    }

    if (energyA <= 0 || energyB <= 0) {
      return 0;
    }

    return correlation / Math.sqrt(energyA * energyB);
  }

  _combScore(envelope, bpm) {
    const lag = (60 / bpm) * this.analysisRate;
    let score = 0;
    let weights = 0;

    for (let harmonic = 1; harmonic <= 4; harmonic++) {
      const harmonicLag = harmonic * lag;

      if (harmonicLag >= envelope.length - 2) {
        break;
      }

      const weight = 1 / Math.sqrt(harmonic);
      score += weight * this._autocorrelation(envelope, harmonicLag);
      weights += weight;
    }

    return weights > 0 ? score / weights : 0;
  }

  _phaseStrength(envelope, periodFrames, offsetFrames) {
    const frames = envelope.length;
    const beatCount = Math.floor((frames - offsetFrames) / periodFrames);

    if (beatCount <= 0) {
      return 0;
    }

    // Small timing jitter should not hide a beat. The radius scales with the
    // period, but stays narrow enough to see genuine half-time gaps.
    const radius = Math.max(1, Math.round(periodFrames * 0.06));
    let total = 0;

    for (let beat = 0; beat < beatCount; beat++) {
      const center = Math.round(offsetFrames + beat * periodFrames);
      let strongest = 0;

      for (let offset = -radius; offset <= radius; offset++) {
        const index = center + offset;

        if (index >= 0 && index < frames && envelope[index] > strongest) {
          strongest = envelope[index];
        }
      }

      total += strongest;
    }

    return total / beatCount;
  }

  _bestPhase(envelope, periodFrames) {
    let bestOffset = 0;
    let bestStrength = -1;
    const steps = 16;

    for (let step = 0; step < steps; step++) {
      const offset = (step / steps) * periodFrames;
      const strength = this._phaseStrength(envelope, periodFrames, offset);

      if (strength > bestStrength) {
        bestStrength = strength;
        bestOffset = offset;
      }
    }

    return { offset: bestOffset, strength: bestStrength };
  }

  _scoreCandidates(envelope) {
    let bestBpm = this.minBpm;
    let bestScore = -Infinity;

    const coarseStep = 1;

    for (let bpm = this.minBpm; bpm <= this.maxBpm; bpm += coarseStep) {
      const score = this._scoreBpm(envelope, bpm);

      if (score > bestScore) {
        bestScore = score;
        bestBpm = bpm;
      }
    }

    const refineStart = Math.max(this.minBpm, bestBpm - coarseStep);
    const refineEnd = Math.min(this.maxBpm, bestBpm + coarseStep);

    for (let bpm = refineStart; bpm <= refineEnd; bpm += 0.05) {
      const score = this._scoreBpm(envelope, bpm);

      if (score > bestScore) {
        bestScore = score;
        bestBpm = bpm;
      }
    }

    return { bpm: bestBpm, score: bestScore };
  }

  _refineWithPhase(envelope, bpm, range = 1.5) {
    // Autocorrelation is robust but its peak can be a fraction of a BPM away
    // from the real tempo. Maximising beat alignment sharpens the estimate
    // without making the global search expensive.
    const start = Math.max(this.minBpm, bpm - range);
    const end = Math.min(this.maxBpm, bpm + range);
    let bestBpm = bpm;
    let bestScore = -Infinity;

    for (let candidate = start; candidate <= end; candidate += 0.05) {
      const periodFrames = (60 / candidate) * this.analysisRate;
      const { strength } = this._bestPhase(envelope, periodFrames);
      const score = strength + this._combScore(envelope, candidate) * 0.01;

      if (score > bestScore) {
        bestScore = score;
        bestBpm = candidate;
      }
    }

    return bestBpm;
  }

  _scoreBpm(envelope, bpm) {
    const score = this._combScore(envelope, bpm);

    // Mild prior: avoid implausible extremes when the evidence is ambiguous.
    const prior = Math.exp(-0.5 * Math.pow(Math.log2(bpm / 120) / 1.0, 2));

    return score * (0.8 + 0.2 * prior);
  }

  _correctOctave(envelope, bpm) {
    // If the midpoint between the detected beats is also strong, the true beat
    // is twice as fast. Genuine half-time patterns leave those midpoints empty.
    const periodFrames = (60 / bpm) * this.analysisRate;
    const { offset } = this._bestPhase(envelope, periodFrames);
    const beatStrength = this._phaseStrength(envelope, periodFrames, offset);
    const midpointStrength = this._phaseStrength(
      envelope,
      periodFrames,
      offset + periodFrames / 2,
    );

    if (beatStrength <= 1e-6) {
      return bpm;
    }

    const midpointRatio = midpointStrength / beatStrength;

    if (midpointRatio > this.octaveMidpointRatio && bpm * 2 <= this.maxBpm) {
      return bpm * 2;
    }

    return bpm;
  }

  _analyze() {
    if (this._samples.length < 4) {
      return;
    }

    const newest = this._samples[this._samples.length - 1].time;
    const oldest = this._samples[0].time;

    if (newest - oldest < this.minHistorySeconds) {
      return;
    }

    const envelope = this._buildEnvelope();
    let activity = 0;

    for (let i = 0; i < envelope.length; i++) {
      activity += envelope[i];
    }

    if (activity <= 1e-6) {
      this.confidence += this.confidenceSmoothing * (0 - this.confidence);

      if (this.confidence < 0.01) {
        this.confidence = 0;
      }

      return;
    }

    const candidate = this._scoreCandidates(envelope);
    const refinedBpm = this._refineWithPhase(envelope, candidate.bpm);
    const correctedBpm = this._correctOctave(envelope, refinedBpm);
    const finalBpm = this._refineWithPhase(envelope, correctedBpm);
    const bpm = clamp(finalBpm, this.minBpm, this.maxBpm);
    const selectedScore = this._combScore(envelope, bpm);
    const activityFactor = Math.min(1, activity / (this.historyFrames * 0.02));
    const rawConfidence = clamp(selectedScore * activityFactor, 0, 1);

    this._estimates.push(bpm);

    if (this._estimates.length > this.smoothingWindow) {
      this._estimates.shift();
    }

    const smoothedBpm = median(this._estimates);

    this.bpm = Math.round(smoothedBpm * 10) / 10;
    this.confidence +=
      this.confidenceSmoothing * (rawConfidence - this.confidence);
    this.confidence = clamp(this.confidence, 0, 1);
  }
}

export function createTempoDetector(options) {
  return new TempoDetector(options);
}

export default TempoDetector;
