const effect = (name, fragmentShader, audio = false) => ({
  meta: {
    name: `Point10 ${name}`,
    author: "Point10",
    version: "1.0.0",
    previewWithOutput: true,
    type: "shader",
    ...(audio ? { audioFeatures: ["bass", "mid", "treble"] } : {}),
  },
  fragmentShader,
});

const noise = `
  float p10Hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
  }

  vec2 p10Hash2(vec2 p) {
    return fract(sin(vec2(
      dot(p, vec2(127.1, 311.7)),
      dot(p, vec2(269.5, 183.3))
    )) * 43758.5453);
  }

  float p10Noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = p10Hash(i);
    float b = p10Hash(i + vec2(1.0, 0.0));
    float c = p10Hash(i + vec2(0.0, 1.0));
    float d = p10Hash(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
  }

  float p10Fbm(vec2 p) {
    float sum = 0.0;
    float amp = 0.5;
    for (int i = 0; i < 4; i++) {
      sum += p10Noise(p) * amp;
      p *= 2.13;
      amp *= 0.5;
    }
    return sum;
  }
`;

const bayer = `
  float p10Bayer2(vec2 p) {
    p = floor(p);
    return fract(p.x / 2.0 + p.y * p.y * 0.75);
  }

  float p10Bayer4(vec2 p) {
    return p10Bayer2(0.5 * p) * 0.25 + p10Bayer2(p);
  }

  float p10Bayer8(vec2 p) {
    return p10Bayer4(0.5 * p) * 0.25 + p10Bayer2(p);
  }
`;

export default [
  effect(
    "Gray",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec4 c = texture(u_modVCanvas, vUv);
        c.rgb = vec3(dot(vec3(0.222, 0.707, 0.071), c.rgb));
        fragColor = c;
      }`,
  ),

  effect(
    "Invert",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec4 c = texture(u_modVCanvas, vUv);
        c.rgb = vec3(1.0) - c.rgb;
        fragColor = c;
      }`,
  ),

  effect(
    "Posterization",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec4 c = texture(u_modVCanvas, vUv);
        float gamma = 0.6;
        float numColors = 4.0;
        vec3 c2 = pow(c.rgb, vec3(gamma));
        c2 = floor(c2 * numColors) / numColors;
        c.rgb = pow(c2, vec3(1.0 / gamma));
        fragColor = c;
      }`,
  ),

  effect(
    "Film Grain",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec2 p = vUv;
        vec4 textureScreen = texture(u_modVCanvas, p);
        float fNintensity = 0.35;
        float fSintensity = 0.35;
        float fScount = 4096.0;
        float x = p.x * p.y * iTime * 1000.0;
        float dx = mod(mod(x, 13.0) * mod(x, 123.0), 0.01);
        vec3 result = textureScreen.rgb + textureScreen.rgb *
          clamp(0.1 + dx * 100.0, 0.0, 1.0);
        vec2 sc = vec2(sin(p.y * fScount), cos(p.y * fScount));
        result += textureScreen.rgb * sc.xyx * fSintensity;
        result = textureScreen.rgb +
          clamp(fNintensity, 0.0, 1.0) * (result - textureScreen.rgb);
        result = vec3(dot(vec3(0.222, 0.707, 0.071), result));
        fragColor = vec4(result, 1.0);
      }`,
  ),

  effect(
    "BlackAndWhite",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec4 c = texture(u_modVCanvas, vUv);
        float t = (c.r + c.g + c.b) / 3.0;
        c.rgb = vec3(1.0);
        if (t < 0.1 || t > 0.9) c.rgb = vec3(0.0);
        fragColor = c;
      }`,
  ),

  effect(
    "Gray Stripes",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec4 c = texture(u_modVCanvas, vUv);
        float aspect = iResolution.x / max(iResolution.y, 1.0);
        if (fract(vUv.x * aspect * 5.0) < 0.5) {
          c.rgb = vec3(dot(vec3(0.222, 0.707, 0.071), c.rgb));
        }
        fragColor = c;
      }`,
  ),

  effect(
    "Invert Gray",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec4 c = texture(u_modVCanvas, vUv);
        c.rgb = vec3(1.0) - c.rgb;
        c.rgb = vec3(dot(vec3(0.222, 0.707, 0.071), c.rgb));
        fragColor = c;
      }`,
  ),

  effect(
    "Crosshatching",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec4 c = texture(u_modVCanvas, vUv);
        float l = length(c.rgb);
        c.rgb = vec3(1.0);
        if (l < 1.0 && mod(fragCoord.x + fragCoord.y, 10.0) < 0.5) {
          c.rgb = vec3(0.0);
        }
        if (l < 0.7 && mod(fragCoord.x - fragCoord.y, 10.0) < 0.5) {
          c.rgb = vec3(0.0);
        }
        if (l < 0.5 && mod(fragCoord.x + fragCoord.y - 5.0, 10.0) < 0.5) {
          c.rgb = vec3(0.0);
        }
        if (l < 0.3 && mod(fragCoord.x - fragCoord.y - 5.0, 10.0) < 0.5) {
          c.rgb = vec3(0.0);
        }
        fragColor = c;
      }`,
  ),

  effect(
    "Blocks",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        float numTiles = 128.0;
        vec2 size = 1.0 / vec2(
          numTiles * (iResolution.x / max(iResolution.y, 1.0)),
          numTiles
        );
        vec2 base = vUv - mod(vUv, size);
        fragColor = texture(u_modVCanvas, base + size * 0.5);
      }`,
  ),

  effect(
    "3d Blocks",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        float numTiles = 70.0;
        float threshold = 0.15;
        vec3 edgeColor = vec3(0.7);
        float size = 1.0 / numTiles;
        vec2 base = vUv - mod(vUv, vec2(size));
        vec2 center = base + vec2(size * 0.5);
        vec2 st = (vUv - base) / size;
        vec4 c1 = vec4(0.0);
        vec4 c2 = vec4(0.0);
        vec4 inverseOffset = vec4(1.0 - edgeColor, 1.0);
        if (st.x > st.y) c1 = inverseOffset;
        if (st.x > 1.0 - threshold) c2 = c1;
        if (st.y > 1.0 - threshold) c2 = c1;
        vec4 bottom = c2;
        c1 = vec4(0.0);
        c2 = vec4(0.0);
        if (st.x > st.y) c1 = inverseOffset;
        if (st.x < threshold) c2 = c1;
        if (st.y < threshold) c2 = c1;
        vec4 top = c2;
        fragColor = texture(u_modVCanvas, center) + top - bottom;
      }`,
  ),

  effect(
    "Bayer 8 Matrix Dithering BW",
    `${bayer}
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec4 c = texture(u_modVCanvas, vUv);
        float grayscale = dot(c.rgb, vec3(0.2126, 0.7152, 0.0722));
        float limit = p10Bayer8(fragCoord);
        float value = grayscale < limit ? 0.0 : 1.0;
        fragColor = vec4(value, value, value, 1.0);
      }`,
  ),

  effect(
    "Bayer Matrix Dithering Color",
    `${bayer}
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec3 rgb = texture(u_modVCanvas, vUv).rgb;
        float limit = p10Bayer4(fragCoord);
        vec3 color = vec3(
          rgb.r < limit ? 0.0 : 1.0,
          rgb.g < limit ? 0.0 : 1.0,
          rgb.b < limit ? 0.0 : 1.0
        );
        fragColor = vec4(color, 1.0);
      }`,
  ),

  effect(
    "Fuzzy Pixels",
    `${noise}
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec2 rnd = vec2(p10Hash(vUv), p10Hash(vUv + 0.1));
        fragColor = texture(u_modVCanvas, vUv + rnd * 0.01);
      }`,
  ),

  effect(
    "Fractured",
    `${noise}
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec2 p = vUv * 10.0;
        vec2 cell = floor(p);
        vec2 local = fract(p);
        vec2 point = vec2(p10Hash(cell), p10Hash(cell + 17.0));
        vec2 delta = normalize(local - point + 0.0001);
        fragColor = texture(u_modVCanvas, fract(vUv + delta * 0.005));
      }`,
  ),

  effect(
    "Bathroom Window",
    `${noise}
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec2 texel = 1.0 / iResolution.xy;
        float center = p10Fbm(vUv * 5.0);
        float ox = p10Fbm((vUv - vec2(texel.x, 0.0)) * 5.0) -
          p10Fbm((vUv + vec2(texel.x, 0.0)) * 5.0);
        float oy = p10Fbm((vUv - vec2(0.0, texel.y)) * 5.0) -
          p10Fbm((vUv + vec2(0.0, texel.y)) * 5.0);
        vec2 offset = vec2(ox, oy);
        vec4 c = texture(u_modVCanvas, vUv + offset * 0.25);
        fragColor = vec4(c.rgb + offset.x * abs(offset.y) * 10.0, 1.0);
      }`,
  ),

  effect(
    "Contrast",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec4 c = texture(u_modVCanvas, vUv);
        c.rgb = c.rgb - vec3(u_bass) * (c.rgb - 1.0) * c.rgb * (c.rgb - 0.5);
        fragColor = c;
      }`,
    true,
  ),

  effect(
    "Color Distort",
    `${noise}
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        float scale = 0.05 / (iResolution.x / max(iResolution.y, 1.0));
        float t = iTime * 0.2;
        float strength1 = min(1.0, u_bass * 5.0);
        float strength2 = min(1.0, u_mid * 5.0);
        vec3 c;
        c.r = texture(u_modVCanvas, vec2(vUv.x + p10Noise(vec2(t, 0.0)) * 0.5 * strength1 * scale, vUv.y)).r;
        c.g = texture(u_modVCanvas, vUv).g;
        c.b = texture(u_modVCanvas, vec2(vUv.x + p10Noise(vec2(t, 2.232)) * 0.5 * strength2 * scale, vUv.y)).b;
        fragColor = vec4(c, 1.0);
      }`,
    true,
  ),

  effect(
    "Crt",
    `
      const float crtHardScan = -8.0;
      const float crtHardPix = -3.0;
      const float crtMaskDark = 0.5;
      const float crtMaskLight = 1.5;

      vec3 crtFetch(vec2 pos, vec2 off) {
        vec2 res = iResolution.xy / 6.0;
        pos = floor(pos * res + off) / res;
        if (max(abs(pos.x - 0.5), abs(pos.y - 0.5)) > 0.5) {
          return vec3(0.0);
        }
        return texture(u_modVCanvas, pos).rgb;
      }

      vec2 crtDist(vec2 pos) {
        vec2 res = iResolution.xy / 6.0;
        pos = pos * res;
        return -((pos - floor(pos)) - vec2(0.5));
      }

      float crtGaus(float pos, float scale) {
        return exp2(scale * pos * pos);
      }

      vec3 crtHorz3(vec2 pos, float off) {
        vec3 b = crtFetch(pos, vec2(-1.0, off));
        vec3 c = crtFetch(pos, vec2(0.0, off));
        vec3 d = crtFetch(pos, vec2(1.0, off));
        float dpos = crtDist(pos).x;
        float wb = crtGaus(dpos - 1.0, crtHardPix);
        float wc = crtGaus(dpos, crtHardPix);
        float wd = crtGaus(dpos + 1.0, crtHardPix);
        return (b * wb + c * wc + d * wd) / (wb + wc + wd);
      }

      vec3 crtHorz5(vec2 pos, float off) {
        vec3 a = crtFetch(pos, vec2(-2.0, off));
        vec3 b = crtFetch(pos, vec2(-1.0, off));
        vec3 c = crtFetch(pos, vec2(0.0, off));
        vec3 d = crtFetch(pos, vec2(1.0, off));
        vec3 e = crtFetch(pos, vec2(2.0, off));
        float dpos = crtDist(pos).x;
        float wa = crtGaus(dpos - 2.0, crtHardPix);
        float wb = crtGaus(dpos - 1.0, crtHardPix);
        float wc = crtGaus(dpos, crtHardPix);
        float wd = crtGaus(dpos + 1.0, crtHardPix);
        float we = crtGaus(dpos + 2.0, crtHardPix);
        return (a * wa + b * wb + c * wc + d * wd + e * we) /
          (wa + wb + wc + wd + we);
      }

      float crtScan(vec2 pos, float off) {
        return crtGaus(crtDist(pos).y + off, crtHardScan);
      }

      vec3 crtTri(vec2 pos) {
        vec3 a = crtHorz3(pos, -1.0);
        vec3 b = crtHorz5(pos, 0.0);
        vec3 c = crtHorz3(pos, 1.0);
        float wa = crtScan(pos, -1.0);
        float wb = crtScan(pos, 0.0);
        float wc = crtScan(pos, 1.0);
        return a * wa + b * wb + c * wc;
      }

      vec3 crtMask(vec2 pos) {
        pos.x += pos.y * 3.0;
        vec3 m = vec3(crtMaskDark);
        pos.x = fract(pos.x / 6.0);
        if (pos.x < 0.333) m.r = crtMaskLight;
        else if (pos.x < 0.666) m.g = crtMaskLight;
        else m.b = crtMaskLight;
        return m;
      }

      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        fragColor = vec4(crtTri(vUv) * crtMask(fragCoord), 1.0);
      }`,
  ),

  effect(
    "Cubic Lens Distortion",
    `
      float cubicRand(vec2 p) {
        return fract(sin(dot(p.xy, vec2(12.9898, 78.233))) * 43758.5453 +
          fract(iTime) * 0.5);
      }

      vec3 cubicBlur(vec2 p) {
        vec2 powers = pow(abs(p - 0.5), vec2(2.0));
        float n = cubicRand(p) * 0.004 *
          smoothstep(0.05, 0.45, powers.x + powers.y) * 5.5;
        vec2 xy1 = p + n;
        vec2 xy2 = p - n;
        return ((texture(u_modVCanvas, xy1) +
          texture(u_modVCanvas, xy2) +
          texture(u_modVCanvas, vec2(xy1.x, xy2.y)) +
          texture(u_modVCanvas, vec2(xy2.x, xy1.y))) / 4.0).rgb;
      }

      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec2 p = vUv * 2.0 - 1.0;
        vec2 rescaleTerm = 0.9 * (vUv - 0.5);
        vec3 c = vec3(
          cubicBlur(1.0675 * rescaleTerm + 0.5).r,
          cubicBlur(1.0450 * rescaleTerm + 0.5).g,
          cubicBlur(1.0225 * rescaleTerm + 0.5).b
        );
        c *= 0.5 + 0.8 * clamp(1.0 - dot(p, p), 0.0, 1.0);
        fragColor = vec4(c, 1.0);
      }`,
  ),

  effect(
    "Hexagons",
    `
      vec4 downsample36(vec2 p) {
        vec2 size = iResolution.xy;
        vec2 ts = 1.0 / size;
        vec2 ts2 = ts * 2.0;
        vec2 uv = p / size;
        vec4 c = texture(u_modVCanvas, uv) * 0.5;
        c += texture(u_modVCanvas, uv + vec2(ts.x, ts.y)) * 0.5;
        c += texture(u_modVCanvas, uv + vec2(-ts.x, ts.y)) * 0.5;
        c += texture(u_modVCanvas, uv + vec2(ts.x, -ts.y)) * 0.5;
        c += texture(u_modVCanvas, uv + vec2(-ts.x, -ts.y)) * 0.5;
        c += texture(u_modVCanvas, uv + vec2(0.0, ts2.y)) * 0.25;
        c += texture(u_modVCanvas, uv + vec2(-ts2.x, 0.0)) * 0.25;
        c += texture(u_modVCanvas, uv + vec2(ts2.x, 0.0)) * 0.25;
        c += texture(u_modVCanvas, uv + vec2(0.0, -ts2.y)) * 0.25;
        c += texture(u_modVCanvas, uv + vec2(ts2.x, ts2.y)) * 0.125;
        c += texture(u_modVCanvas, uv + vec2(-ts2.x, ts2.y)) * 0.125;
        c += texture(u_modVCanvas, uv + vec2(ts2.x, -ts2.y)) * 0.125;
        c += texture(u_modVCanvas, uv + vec2(-ts2.x, -ts2.y)) * 0.125;
        return c * 0.25;
      }

      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        float sqrt3 = 1.7320508;
        float halfSqrt3 = 0.8660254;
        float size = 10.0;
        vec2 grid = vec2(size, sqrt3 * size);
        vec2 base = iResolution.xy * 0.5;
        vec2 fc = fragCoord;
        vec2 po1 = floor((fc - base) / grid + 0.5) * grid + base - fc;
        float dist1 = length(po1);
        vec2 base2 = base + size * vec2(0.5, halfSqrt3);
        vec2 po2 = floor((fc - base2) / grid + 0.5) * grid + base2 - fc;
        float dist2 = length(po2);
        vec2 sp = dist1 < dist2 ? po1 : po2;
        fragColor = downsample36(sp + fc);
      }`,
  ),

  effect(
    "Led Display",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec2 size = iResolution.xy;
        vec2 ledSize = 1.0 / size;
        vec2 ledCenter = floor(vUv * size) / size + ledSize * 0.5;
        vec4 c = texture(u_modVCanvas, ledCenter);
        float luminance = (c.r + c.g + c.b) / 3.0;
        if (luminance > 0.01) {
          vec3 ratios = c.rgb / luminance;
          float stepSize = 1.0 / 100.0;
          float factor = stepSize * floor(luminance / stepSize);
          c.rgb = ratios * factor * 1.5;
        }
        float v = length((ledCenter - vUv) / (ledSize * 0.5));
        fragColor = mix(c, vec4(0.0, 0.0, 0.0, 1.0), v);
      }`,
  ),

  effect(
    "Led Display Curved",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec2 p = vUv * 2.0 - 1.0;
        vec2 bend = vUv + p * vec2(p.x * p.x, p.y * p.y) * 0.04;
        vec2 size = iResolution.xy;
        vec2 ledSize = 1.0 / size;
        vec2 ledCenter = floor(bend * size) / size + ledSize * 0.5;
        vec4 c = texture(u_modVCanvas, ledCenter);
        vec2 lpos = (ledCenter - bend) / (ledSize * 0.5);
        float v = 1.0 - smoothstep(0.1, 1.0,
          lpos.x * lpos.x + lpos.y * lpos.y - lpos.x * lpos.x * lpos.y * lpos.y);
        c = mix(vec4(0.0, 0.0, 0.0, 1.0), c, v);
        c = mix(vec4(0.0, 0.0, 0.0, 1.0), c, pow(1.0 - abs(p.y), 1.8));
        fragColor = c;
      }`,
  ),

  effect(
    "Limit Hue",
    `
      float minChannel(vec3 v) { return min(min(v.x, v.y), v.z); }
      float maxChannel(vec3 v) { return max(max(v.x, v.y), v.z); }

      vec3 rgbToHsv(vec3 rgb) {
        vec3 hsv = vec3(0.0);
        float minValue = minChannel(rgb);
        float maxValue = maxChannel(rgb);
        float delta = maxValue - minValue;
        hsv.z = maxValue;
        if (delta != 0.0) {
          hsv.y = delta / maxValue;
          vec3 delRgb = (((vec3(maxValue) - rgb) / 6.0) +
            (delta / 2.0)) / delta;
          if (rgb.x == maxValue) hsv.x = delRgb.z - delRgb.y;
          else if (rgb.y == maxValue) hsv.x = 1.0 / 3.0 + delRgb.x - delRgb.z;
          else hsv.x = 2.0 / 3.0 + delRgb.y - delRgb.x;
          if (hsv.x < 0.0) hsv.x += 1.0;
          if (hsv.x > 1.0) hsv.x -= 1.0;
        }
        return hsv;
      }

      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec4 c = texture(u_modVCanvas, vUv);
        float minHue = fract(iTime * 0.075);
        float maxHue = fract(minHue + 0.2);
        vec3 hsv = rgbToHsv(c.rgb);
        if (maxHue > minHue) {
          if (hsv.x < minHue || hsv.x > maxHue) {
            c.rgb = vec3(dot(vec3(0.222, 0.707, 0.071), c.rgb));
          }
        } else if (hsv.x > maxHue && hsv.x < minHue) {
          c.rgb = vec3(dot(vec3(0.222, 0.707, 0.071), c.rgb));
        }
        fragColor = c;
      }`,
  ),

  effect(
    "Old Movie",
    `${noise}
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec4 c = texture(u_modVCanvas, vUv);
        float gray = dot(vec3(0.3, 0.59, 0.11), c.rgb);
        vec3 muted = mix(c.rgb, vec3(gray), 0.5);
        vec3 sepia = mix(vec3(0.1, 0.05, 0.0), vec3(1.0, 0.9, 0.5), gray);
        c.rgb = sepia;
        vec2 s = vUv * vec2(3.0, 80.0) + vec2(iTime * 0.03, iTime * 0.03);
        float scratch = p10Hash(floor(s * vec2(80.0, 1.0)));
        scratch = smoothstep(0.985, 1.0, scratch);
        fragColor = c + vec4(vec3(scratch), 0.0);
      }`,
  ),

  effect(
    "Embos Merge",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec4 c = texture(u_modVCanvas, vUv);
        vec2 texel = 1.0 / iResolution.xy;
        vec3 emboss = texture(u_modVCanvas, vUv - texel).rgb * 2.0 -
          texture(u_modVCanvas, vUv + texel).rgb * 2.0;
        c.rgb += vec3((emboss.r + emboss.g + emboss.b) / 3.0);
        fragColor = c;
      }`,
  ),

  effect(
    "Embos",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec2 texel = 1.0 / iResolution.xy;
        vec3 emboss = texture(u_modVCanvas, vUv - texel).rgb * 2.0 -
          texture(u_modVCanvas, vUv + texel).rgb * 2.0;
        float value = (emboss.r + emboss.g + emboss.b) / 3.0;
        vec4 base = texture(u_modVCanvas, vUv);
        fragColor = vec4(base.rgb + vec3(value) + vec3(value) * 0.6, 1.0);
      }`,
  ),

  effect(
    "Pixelation",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec2 cells = max(vec2(1.0), floor(iResolution.xy / 8.0));
        vec2 uv = (floor(vUv * cells) + 0.5) / cells;
        fragColor = texture(u_modVCanvas, uv);
      }`,
  ),

  effect(
    "RGB Shift",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        float amount = 0.0015 + min(0.007, u_mid * 0.01);
        vec2 offset = vec2(cos(amount * 1200.0), sin(amount * 900.0)) /
          iResolution.xy * 4.0;
        vec4 center = texture(u_modVCanvas, vUv);
        float red = texture(u_modVCanvas, vUv + offset).r;
        float blue = texture(u_modVCanvas, vUv - offset).b;
        fragColor = vec4(red, center.g, blue, center.a);
      }`,
    true,
  ),

  effect(
    "Digital Glitch",
    `${noise}
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        float amount = min(
          0.09,
          0.012 + float(u_kick) * 0.035 + u_bass * 0.02
        );
        float frame = floor(iTime * 14.0);
        float row = floor(vUv.y * 32.0);
        float burst = step(0.82, p10Hash(vec2(row, frame)));
        vec2 uv = vUv;
        uv.x += (p10Hash(vec2(row, frame + 1.0)) - 0.5) * amount * burst;
        vec4 color = texture(u_modVCanvas, uv);
        float split = amount * (0.35 + p10Hash(vec2(row, frame + 2.0)) * 0.65);
        color.r = texture(u_modVCanvas, uv + vec2(split, 0.0)).r;
        color.b = texture(u_modVCanvas, uv - vec2(split, 0.0)).b;
        fragColor = color;
      }`,
    true,
  ),

  effect(
    "Vignette",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec4 color = texture(u_modVCanvas, vUv);
        vec2 p = vUv * 2.0 - 1.0;
        float edge = smoothstep(0.35, 1.45, dot(p, p));
        fragColor = vec4(color.rgb * (1.0 - edge * 0.72), color.a);
      }`,
  ),

  effect(
    "Bloom",
    `
      void mainImage(out vec4 fragColor, in vec2 fragCoord) {
        vec4 base = texture(u_modVCanvas, vUv);
        vec2 texel = 1.0 / iResolution.xy;
        vec3 glow = vec3(0.0);
        float weight = 0.0;
        for (int x = -2; x <= 2; x++) {
          for (int y = -2; y <= 2; y++) {
            vec2 offset = vec2(float(x), float(y)) * texel * 3.0;
            vec3 sampleColor = texture(u_modVCanvas, vUv + offset).rgb;
            float brightness = max(sampleColor.r,
              max(sampleColor.g, sampleColor.b));
            float w = smoothstep(0.35, 0.9, brightness) /
              (1.0 + float(x * x + y * y));
            glow += sampleColor * w;
            weight += w;
          }
        }
        glow /= max(weight, 0.0001);
        fragColor = vec4(base.rgb + glow * 0.72, 1.0);
      }`,
  ),
];
