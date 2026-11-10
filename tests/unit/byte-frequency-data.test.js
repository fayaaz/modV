import { describe, expect, it } from "vitest";
import { reduceByteFrequencyData } from "../../src/renderer/src/application/byte-frequency-data.js";

describe("reduceByteFrequencyData", () => {
  it("returns an empty array for empty or missing input", () => {
    expect(reduceByteFrequencyData(undefined, 512)).toEqual(new Uint8Array(0));
    expect(reduceByteFrequencyData(null, 512)).toEqual(new Uint8Array(0));
    expect(reduceByteFrequencyData(new Uint8Array(0), 512)).toEqual(
      new Uint8Array(0),
    );
  });

  it("returns a copy of the input when it already fits the target", () => {
    const input = new Uint8Array([1, 2, 3, 4]);
    const reduced = reduceByteFrequencyData(input, 4);

    expect(reduced).toEqual(input);
    expect(reduced).not.toBe(input);
  });

  it("returns a copy of the input when it is shorter than the target", () => {
    const input = new Uint8Array([10, 20, 30]);
    expect(reduceByteFrequencyData(input, 512)).toEqual(input);
  });

  it("merges adjacent bin pairs when halving 1024 bins to 512", () => {
    const input = new Uint8Array(1024);

    for (let i = 0; i < input.length; i += 1) {
      input[i] = i % 256;
    }

    // Put a peak in an "even" bin and a bigger peak in its odd neighbour:
    // the segment max must win either way around.
    input[100] = 7;
    input[101] = 250;
    input[202] = 250;
    input[203] = 7;

    const reduced = reduceByteFrequencyData(input, 512);

    expect(reduced).toBeInstanceOf(Uint8Array);
    expect(reduced).toHaveLength(512);

    for (let i = 0; i < reduced.length; i += 1) {
      expect(reduced[i]).toBe(Math.max(input[i * 2], input[i * 2 + 1]));
    }

    expect(reduced[50]).toBe(250);
    expect(reduced[101]).toBe(250);
  });

  it("partitions the source into floor-bounded segments for non-divisible ratios", () => {
    // 10 bins -> 4 segments: [0,2) [2,5) [5,7) [7,10)
    const input = new Uint8Array([10, 90, 30, 10, 80, 5, 60, 20, 70, 40]);
    const reduced = reduceByteFrequencyData(input, 4);

    expect(Array.from(reduced)).toEqual([90, 80, 60, 70]);
  });

  it("never reads bins outside the source array", () => {
    const input = new Uint8Array(3).fill(255);
    const reduced = reduceByteFrequencyData(input, 2);

    // Segments [0,1) and [1,3) - all in bounds.
    expect(Array.from(reduced)).toEqual([255, 255]);
  });

  it("keeps integer byte values", () => {
    const input = new Uint8Array(1024).fill(255);
    const reduced = reduceByteFrequencyData(input, 512);

    for (const value of reduced) {
      expect(Number.isInteger(value)).toBe(true);
      expect(value).toBeLessThanOrEqual(255);
    }
  });
});
