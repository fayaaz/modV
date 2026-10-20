/*{
  "CREDIT": "Point10",
  "DESCRIPTION": "Transition imported from Point10",
  "ISFVSN": "2",
  "CATEGORIES": ["Transition"],
  "INPUTS": [
    { "NAME": "startImage", "TYPE": "image" },
    { "NAME": "endImage", "TYPE": "image" },
    { "NAME": "progress", "TYPE": "float", "MIN": 0.0, "MAX": 1.0, "DEFAULT": 0.0 }
  ]
}*/

#define PI 3.14159265358979323846
#define PI2 6.28318530717958623200
#define saturate(a) clamp(a, 0.0, 1.0)

float p10Hash(vec2 p) {
  return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
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

float _rand(vec2 p) {
  return p10Hash(p);
}

float _noise(vec2 p) {
  return p10Noise(p);
}

float _noise(vec3 p) {
  return p10Noise(p.xy + p.z * 17.0);
}

float _turbulence(vec2 p, float octaves) {
  float sum = 0.0;
  float ampSum = 0.0;
  float freq = 1.0;
  float amp = 1.0;

  for (int i = 0; i < 6; i++) {
    if (float(i) >= octaves) {
      break;
    }

    sum += abs(_noise(p * freq)) * amp;
    ampSum += amp;
    freq *= 2.13628142;
    amp *= 0.5;
  }

  return sum / ampSum;
}

float _turbulence(vec3 p, float octaves) {
  return _turbulence(p.xy + p.z * 7.0, octaves);
}

float _turbulencefast(vec2 p, float octaves) {
  return _turbulence(p, octaves);
}

float _turbulencefast(vec3 p, float octaves) {
  return _turbulence(p, octaves);
}

float _bump(float value, float start, float end, float width) {
  return smoothstep(start - width, start, value) -
    smoothstep(end, end + width, value);
}




vec4 SampleWithBorder(vec4 border, vec2 uv)
{
	vec2 t = abs(saturate(uv) - uv);
	if (t.x > 0.99 || t.y > 0.99)
		return border;
	return vec4(texture2D(startImage, uv).rgb, 1.0);
}


void main()
{
	float offset = 0.1+(_noise(vec2(isf_FragNormCoord.x * 5.0, 0.5))*0.5+0.5);
	vec4 c2 = texture2D(endImage, isf_FragNormCoord);
	
	if ((1.0-isf_FragNormCoord.y) < offset * progress)
	{
		gl_FragColor = c2;	
		return;
	}
	
	vec4 c1 = SampleWithBorder(vec4(0.0), vec2(isf_FragNormCoord.x, isf_FragNormCoord.y + offset * progress));
	gl_FragColor = mix(c1, c2, progress);
}

