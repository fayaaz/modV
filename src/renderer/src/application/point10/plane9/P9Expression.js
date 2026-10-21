// Minimal value-noise helper extracted from Point10's plane9/P9Expression.js.
// AroundPlus only evaluates noise1 for the camera roll; the rest of the
// expression parser is not vendored.

const seededRand = (a = 0, b = 0) => {
  let h = (a | 0) * 374761393 + (b | 0) * 668265263;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  h ^= h >>> 16;
  return (h >>> 0) / 4294967296;
};

const hash2 = (x, y) => seededRand(x, y);

const smooth = (t) => t * t * (3 - 2 * t);

export function valueNoise(dim, ...p) {
  const n = dim === 1 ? 1 : dim === 2 ? 2 : 3;
  const f = p.map((v) => v - Math.floor(v));
  const i = p.map((v) => Math.floor(v));
  const s = f.map(smooth);
  if (n === 1) {
    const a = hash2(i[0], 1);
    const b = hash2(i[0] + 1, 1);
    return (a + (b - a) * s[0]) * 2 - 1;
  }
  if (n === 2) {
    const vs = [0, 1];
    return (
      vs.reduce(
        (acc, dy) =>
          acc +
          vs.reduce((acc2, dx) => {
            const v = hash2(i[0] + dx, i[1] + dy);
            return acc2 + v * (dx ? s[0] : 1 - s[0]) * (dy ? s[1] : 1 - s[1]);
          }, 0),
        0,
      ) *
        2 -
      1
    );
  }
  const vs = [0, 1];
  return (
    vs.reduce(
      (acc, dz) =>
        acc +
        vs.reduce(
          (acc2, dy) =>
            acc2 +
            vs.reduce((acc3, dx) => {
              const v = hash2(i[0] + dx, hash2(i[1] + dy, i[2] + dz));
              const w =
                (dx ? s[0] : 1 - s[0]) *
                (dy ? s[1] : 1 - s[1]) *
                (dz ? s[2] : 1 - s[2]);
              return acc3 + v * w;
            }, 0),
          0,
        ),
      0,
    ) *
      2 -
    1
  );
}
