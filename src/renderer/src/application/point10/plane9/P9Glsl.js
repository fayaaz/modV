// The Plane9 common GLSL preamble, extracted verbatim from Point10's
// plane9/P9Glsl.js. AroundPlus's concrete shader only needs this constant;
// the full shader compiler is not vendored.

export const COMMON_PREAMBLE = `
#define PI 3.14159265358979323846
#define PI2 6.28318530717958623200
#ifndef saturate
// plane9 shaders call saturate(x) on float/vec2/vec3/vec4 - one arg macros
// cover all four. #ifndef matches three's own prefix under tone mapping; a
// FUNCTION declaration named saturate would be macro-expanded into
// "float clamp(float v, 0.0, 1.0)" -> GLSL '0.0' syntax error.
#define saturate( a ) clamp( a, 0.0, 1.0 )
#endif
// plane9 shaders sample a texture with a bare float coordinate (1D row lookup,
// e.g. Solar Eclipse's texture(gTexture1, si.tex.x) gradient ramp). WebGL2
// GLSL has no texture(sampler2D, float) builtin overload, but user-defined
// overloads of texture() with a different signature are legal ES 3.00 (so a
// helper fn named texture is fine; only saturate collides with three's macro).
vec4 texture(sampler2D t, float x) { return texture(t, vec2(x, 0.5)); }
float _luminance(vec3 v) { return dot(v, vec3(0.2126, 0.7152, 0.0722)); }
vec3 _tosrgb(vec3 v) { return max(vec3(1.055) * pow(v, vec3(0.416666667)) - vec3(0.055), 0.0); }
vec4 _tosrgb(vec4 v) { return vec4(_tosrgb(v.rgb), v.a); }
vec3 _tolinear(vec3 sRGB) { return sRGB * (sRGB * (sRGB * vec3(0.305306011) + vec3(0.682171111)) + vec3(0.012522878)); }
vec4 _tolinear(vec4 sRGB) { return vec4(_tolinear(sRGB.rgb), sRGB.a); }
vec3 _rotateVector(vec3 v, vec4 q)
{
	vec4 i = vec4(q.www*v.xyz + q.yzx*v.zxy - q.zxy*v.yzx, -q.x*v.x - q.y*v.y - q.z*v.z);
	return vec3(i.xyz*q.www + (i.www*-q.xyz) + (i.yzx*-q.zxy) - (i.zxy*-q.yzx));
}
vec4 _axisAngleToQuaternion(vec3 axis, float angle)
{
	float halfAngle = angle*0.5;
	return vec4(axis.xyz * sin(halfAngle), cos(halfAngle));
}
vec3 _hsv2rgb(float h, float s, float v) { return mix(vec3(1.0),saturate((abs(fract(vec3(h)+vec3(3.0,2.0,1.0)/vec3(3.0))*vec3(6.0)-vec3(3.0))-vec3(1.0))),s)*vec3(v); }
vec3 _hsv2rgb(vec3 hsv) { return _hsv2rgb(hsv.x,hsv.y,hsv.z); }
float _rand(float p)	{	vec2 p2 = fract(vec2(p*5.3987,p*5.4421));		p2 += dot(p2.yx,p2.xy + vec2(21.5351,14.3137));				return fract(p2.x*p2.y*95.4307);	}
float _rand(vec2 p)		{	p  = fract(p*vec2(5.3987,5.4421));				p += dot(p.yx,p.xy + vec2(21.5351,14.3137));				return fract(p.x*p.y*95.4307);		}
float _rand(vec3 p)		{  	p  = fract(p*vec3(5.3987,5.4472,6.9371)); 		p += dot( p.yzx,p.xyz + vec3(21.5351,14.3137,15.3247) );    return fract((p.x*p.y + p.z)*95.4307 );	}
vec2 _rand2(vec2 p)		{	p  = fract(p*vec2(5.3987,5.4421));		    	p += dot(p.yx,p.xy +  vec2(21.5351,14.3137));				return fract(vec2(p.x*p.y*95.4307,p.y*p.x*97.5901));	}
vec2 _rand2(vec3 p)		{	p  = fract(p*vec3(5.3987,5.4472,6.9371));		p += dot(p.yzx,p.xyz + vec3(21.5351,14.3137,15.3247));		return fract(vec2(p.x*p.y*p.z*0.954307,p.x*p.y*p.z*0.975901 ));	}
vec3 _rand3(float p)	{	vec3 p2 = fract(p*vec3(5.3987,5.4421,6.9371));	p2 += dot(p2.yzx,p2.xyz + vec3(21.5351,14.3137,15.3247));	return fract(vec3(p2.x*p2.z*95.4307,p2.x*p2.y*97.5901,p2.y*p2.z*93.8369));	}
vec3 _rand3(vec2 p)		{	p  = fract(p*vec2(5.3987,5.4421));		    	p += dot(p.yx,p.xy +  vec2(21.5351,14.3137));				return fract(vec3(p.x*p.y*95.4307,p.x*p.y*97.5901,p.x*p.y*93.8369));	}
vec3 _rand3(vec3 p)		{	p  = fract(p*vec3(5.3987,5.4421,6.9371));		p += dot(p.yzx,p.xyz  + vec3(21.5351,14.3137,15.3247));		return fract(vec3(p.x*p.z*95.4307,p.x*p.y*97.5901,p.y*p.z*93.8369));	}
vec4 _rand4(vec2 p)		{	p  = fract(p*vec2(5.3987,5.4421));				p += dot(p.yx,p.xy +  vec2(21.5351,14.3137));				return fract(vec4(p.x*p.y*95.4307,p.x*p.y*97.5901,p.x*p.y*93.8369,p.x*p.y*91.6931 ));	}

vec3 _randUnitVector(float s)
{
	vec3 v;
	v.y = sin(s);
	v.x = cos(s) * sin(s * v.y);
	v.z = cos(s) * cos(s * v.y);
	return v;
}
vec4 _perm(float pos)	{ return _rand4(vec2(pos, pos*1.1737)); }
vec4 _perm(vec2 pos)	{ return _rand4(pos); }
float _noise(vec3 P);
float _noisefast(vec3 p)  { return _noise(p); }
float _noisefast(vec2 p)  { return _noisefast(vec3(p, 0.0)); }
float _noisefast(float p)   { return _noisefast(vec3(p, 0.0, 0.0)); }
vec3 _noisegradientfast(vec3 p, float d)
{
	float f0 = _noisefast(p);
	float fx = _noisefast(p + vec3(d, 0.0, 0.0));
	float fy = _noisefast(p + vec3(0.0, d, 0.0));
	float fz = _noisefast(p + vec3(0.0, 0.0, d));
	return vec3(fx - f0, fy - f0, fz - f0) / d;
}
vec3 _noisegradientfast(vec2 p, float d)  { return _noisegradientfast(vec3(p, 0.0), d); }
vec3 _noisegradientfast(float p, float d)   { return _noisegradientfast(vec3(p, 0.0, 0.0), d); }
float _fbmfast(vec3 p, float octaves)
{
	const float lacunarity = 2.13628142, gain = 0.5;
	float freq = 1.0, amp = 1.0, sum = 0.0, ampSum = 0.0;
	for(int i=0; i<int(octaves); i++) {
		sum += _noisefast(p*freq)*amp;
		ampSum += amp;
		freq *= lacunarity;
		amp *= gain;
	}
	return sum / ampSum;
}
float _fbmfast(vec2 p, float octaves)  { return _fbmfast(vec3(p, 0.0), octaves); }
float _fbmfast(float p, float octaves)   { return _fbmfast(vec3(p, 0.0, 0.0), octaves); }
float _turbulencefast(vec3 p, float octaves)
{
	const float lacunarity = 2.13628142, gain = 0.5;
	float sum = 0.0, ampSum = 0.0, freq = 1.0, amp = 1.0;
	for(int i=0; i<int(octaves); i++) {
		sum += abs(_noisefast(p*freq))*amp;
		ampSum += amp;
		freq *= lacunarity;
		amp *= gain;
	}
	return sum / ampSum;
}
float _turbulencefast(vec2 p, float octaves)  { return _turbulencefast(vec3(p, 0.0), octaves); }
float _turbulencefast(float p, float octaves)   { return _turbulencefast(vec3(p, 0.0, 0.0), octaves); }
float _ridgedmffast(vec3 p, float octaves)
{
	const float lacunarity = 2.13628142, gain = 0.5, offset = 1.0;
	float sum = 0.0, ampSum = 0.0, freq = 1.0, amp = 0.5, prev = 1.0;
	for(int i=0; i<int(octaves); i++) {
		float n = offset - abs(_noisefast(p*freq));
		n = n*n;
		sum += n*amp*prev;
		ampSum += amp;
		prev = n;
		freq *= lacunarity;
		amp *= gain;
	}
	return sum / ampSum;
}
float _ridgedmffast(vec2 p, float octaves)  { return _ridgedmffast(vec3(p, 0.0), octaves); }
float _ridgedmffast(float p, float octaves)   { return _ridgedmffast(vec3(p, 0.0, 0.0), octaves); }
// By Brian Sharpe (https://github.com/BrianSharpe). License "I'm not one for copyrights. Use the code however you wish."
float _noise(vec2 P)
{
    const float SKEWFACTOR = 0.36602540378443864676372317075294; // 0.5*(sqrt(3.0)-1.0)
    const float UNSKEWFACTOR = 0.21132486540518711774542560974902; // (3.0-sqrt(3.0))/6.0
    const vec3 SIMPLEX_POINTS = vec3( 1.0-UNSKEWFACTOR, -UNSKEWFACTOR, 1.0-2.0*UNSKEWFACTOR ); // simplex triangle geo
    vec2 Pi = floor( P + dot( P, vec2( SKEWFACTOR ) ) );
    vec4 Pt = vec4( Pi.xy, Pi.xy + 1.0 );
    Pt = Pt - floor(Pt * ( 1.0 / 71.0 )) * 71.0;
    Pt += vec2( 26.0, 161.0 ).xyxy;
    Pt *= Pt;
    Pt = Pt.xzxz * Pt.yyww;
    vec4 hash_x = fract( Pt * ( 1.0 / 951.135664 ) );
    vec4 hash_y = fract( Pt * ( 1.0 / 642.949883 ) );
    vec2 v0 = Pi - dot( Pi, vec2( UNSKEWFACTOR ) ) - P;
    vec4 v1pos_v1hash = (v0.x < v0.y) ? vec4(SIMPLEX_POINTS.xy, hash_x.y, hash_y.y) : vec4(SIMPLEX_POINTS.yx, hash_x.z, hash_y.z);
    vec4 v12 = vec4( v1pos_v1hash.xy, SIMPLEX_POINTS.zz ) + v0.xyxy;
    vec3 grad_x = vec3( hash_x.x, v1pos_v1hash.z, hash_x.w ) - 0.49999;
    vec3 grad_y = vec3( hash_y.x, v1pos_v1hash.w, hash_y.w ) - 0.49999;
    vec3 grad_results = inversesqrt( grad_x * grad_x + grad_y * grad_y ) * ( grad_x * vec3( v0.x, v12.xz ) + grad_y * vec3( v0.y, v12.yw ) );
    vec3 m = vec3( v0.x, v12.xz ) * vec3( v0.x, v12.xz ) + vec3( v0.y, v12.yw ) * vec3( v0.y, v12.yw );
    m = max(0.5 - m, 0.0);
    m = m*m;
    return dot(m*m, grad_results) * 99.204334582718712976990005025589;
}
// By Brian Sharpe (https://github.com/BrianSharpe). License "I'm not one for copyrights. Use the code however you wish."
float _noise(vec3 P)
{
    const float SKEWFACTOR = 1.0/3.0;
    const float UNSKEWFACTOR = 1.0/6.0;
    const float SIMPLEX_CORNER_POS = 0.5;
    vec3 Pi = floor( P + dot( P, vec3( SKEWFACTOR) ) );
    vec3 x0 = P - Pi + dot(Pi, vec3( UNSKEWFACTOR ) );
    vec3 g = step(x0.yzx, x0.xyz);
    vec3 l = 1.0 - g;
    vec3 Pi_1 = min( g.xyz, l.zxy );
    vec3 Pi_2 = max( g.xyz, l.zxy );
    vec3 x1 = x0 - Pi_1 + UNSKEWFACTOR;
    vec3 x2 = x0 - Pi_2 + SKEWFACTOR;
    vec3 x3 = x0 - SIMPLEX_CORNER_POS;
    vec4 v1234_x = vec4( x0.x, x1.x, x2.x, x3.x );
    vec4 v1234_y = vec4( x0.y, x1.y, x2.y, x3.y );
    vec4 v1234_z = vec4( x0.z, x1.z, x2.z, x3.z );
    Pi.xyz = Pi.xyz - floor(Pi.xyz * ( 1.0 / 69.0 )) * 69.0;
    vec3 Pi_inc1 = step( Pi, vec3( 69.0 - 1.5 ) ) * ( Pi + 1.0 );
    vec4 Pt = vec4( Pi.xy, Pi_inc1.xy ) + vec2( 50.0, 161.0 ).xyxy;
    Pt *= Pt;
    vec4 V1xy_V2xy = mix( Pt.xyxy, Pt.zwzw, vec4( Pi_1.xy, Pi_2.xy ) );
    Pt = vec4( Pt.x, V1xy_V2xy.xz, Pt.z ) * vec4( Pt.y, V1xy_V2xy.yw, Pt.w );
    const vec3 SOMELARGEFLOATS = vec3( 635.298681, 682.357502, 668.926525 );
    const vec3 ZINC = vec3( 48.500388, 65.294118, 63.934599 );
    vec3 lowz_mods = vec3( 1.0 / ( SOMELARGEFLOATS.xyz + Pi.zzz * ZINC.xyz ) );
    vec3 highz_mods = vec3( 1.0 / ( SOMELARGEFLOATS.xyz + Pi_inc1.zzz * ZINC.xyz ) );
    Pi_1 = ( Pi_1.z < 0.5 ) ? lowz_mods : highz_mods;
    Pi_2 = ( Pi_2.z < 0.5 ) ? lowz_mods : highz_mods;
    vec4 hash_0 = fract( Pt * vec4( lowz_mods.x, Pi_1.x, Pi_2.x, highz_mods.x ) ) - 0.49999;
    vec4 hash_1 = fract( Pt * vec4( lowz_mods.y, Pi_1.y, Pi_2.y, highz_mods.y ) ) - 0.49999;
    vec4 hash_2 = fract( Pt * vec4( lowz_mods.z, Pi_1.z, Pi_2.z, highz_mods.z ) ) - 0.49999;
    vec4 grad_results = inversesqrt( hash_0 * hash_0 + hash_1 * hash_1 + hash_2 * hash_2 ) * ( hash_0 * v1234_x + hash_1 * v1234_y + hash_2 * v1234_z );
    vec4 kernel_weights = v1234_x * v1234_x + v1234_y * v1234_y + v1234_z * v1234_z;
    kernel_weights = max(0.5 - kernel_weights, 0.0);
    kernel_weights = kernel_weights*kernel_weights*kernel_weights;
    return dot( kernel_weights, grad_results ) * 37.837227241611314102871574478976;
}
// By Brian Sharpe (https://github.com/BrianSharpe). License "I'm not one for copyrights. Use the code however you wish."
vec3 _noisederiv(vec2 P)
{
    const float SKEWFACTOR = 0.36602540378443864676372317075294;            // 0.5*(sqrt(3.0)-1.0)
    const float UNSKEWFACTOR = 0.21132486540518711774542560974902;          // (3.0-sqrt(3.0))/6.0
    const vec3 SIMPLEX_POINTS = vec3( 1.0-UNSKEWFACTOR, -UNSKEWFACTOR, 1.0-2.0*UNSKEWFACTOR );  //  simplex triangle geo
    vec2 Pi = floor( P + dot( P, vec2( SKEWFACTOR ) ) );
    vec4 Pt = vec4( Pi.xy, Pi.xy + 1.0 );
    Pt = Pt - floor(Pt * ( 1.0 / 71.0 )) * 71.0;
    Pt += vec2( 26.0, 161.0 ).xyxy;
    Pt *= Pt;
    Pt = Pt.xzxz * Pt.yyww;
    vec4 hash_x = fract( Pt * ( 1.0 / 951.135664 ) );
    vec4 hash_y = fract( Pt * ( 1.0 / 642.949883 ) );
    vec2 v0 = Pi - dot( Pi, vec2( UNSKEWFACTOR ) ) - P;
    vec4 v1pos_v1hash = (v0.x < v0.y) ? vec4(SIMPLEX_POINTS.xy, hash_x.y, hash_y.y) : vec4(SIMPLEX_POINTS.yx, hash_x.z, hash_y.z);
    vec4 v12 = vec4( v1pos_v1hash.xy, SIMPLEX_POINTS.zz ) + v0.xyxy;
    vec3 grad_x = vec3( hash_x.x, v1pos_v1hash.z, hash_x.w ) - 0.49999;
    vec3 grad_y = vec3( hash_y.x, v1pos_v1hash.w, hash_y.w ) - 0.49999;
    vec3 norm = inversesqrt( grad_x * grad_x + grad_y * grad_y );
    grad_x *= norm;
    grad_y *= norm;
    vec3 grad_results = grad_x * vec3( v0.x, v12.xz ) + grad_y * vec3( v0.y, v12.yw );
    vec3 m = vec3( v0.x, v12.xz ) * vec3( v0.x, v12.xz ) + vec3( v0.y, v12.yw ) * vec3( v0.y, v12.yw );
    m = max(0.5 - m, 0.0);
    vec3 m2 = m*m;
    vec3 m4 = m2*m2;
    vec3 temp = 8.0 * m2 * m * grad_results;
    float xderiv = dot( temp, vec3( v0.x, v12.xz ) ) - dot( m4, grad_x );
    float yderiv = dot( temp, vec3( v0.y, v12.yw ) ) - dot( m4, grad_y );
    return vec3( dot( m4, grad_results ), xderiv, yderiv ) * 99.204334582718712976990005025589;
}
// By Brian Sharpe (https://github.com/BrianSharpe). License "I'm not one for copyrights. Use the code however you wish."
vec4 _noisederiv(vec3 P)
{
    const float SKEWFACTOR = 1.0/3.0;
    const float UNSKEWFACTOR = 1.0/6.0;
    const float SIMPLEX_CORNER_POS = 0.5;
	vec3 Pi = floor( P + dot( P, vec3( SKEWFACTOR) ) );
	vec3 x0 = P - Pi + dot(Pi, vec3( UNSKEWFACTOR ) );
	vec3 g = step(x0.yzx, x0.xyz);
	vec3 l = 1.0 - g;
	vec3 Pi_1 = min( g.xyz, l.zxy );
	vec3 Pi_2 = max( g.xyz, l.zxy );
	vec3 x1 = x0 - Pi_1 + UNSKEWFACTOR;
	vec3 x2 = x0 - Pi_2 + SKEWFACTOR;
	vec3 x3 = x0 - SIMPLEX_CORNER_POS;
	vec4 v1234_x = vec4( x0.x, x1.x, x2.x, x3.x );
	vec4 v1234_y = vec4( x0.y, x1.y, x2.y, x3.y );
	vec4 v1234_z = vec4( x0.z, x1.z, x2.z, x3.z );
	Pi.xyz = Pi.xyz - floor(Pi.xyz * ( 1.0 / 69.0 )) * 69.0;
	vec3 Pi_inc1 = step( Pi, vec3( 69.0 - 1.5 ) ) * ( Pi + 1.0 );
	vec4 Pt = vec4( Pi.xy, Pi_inc1.xy ) + vec2( 50.0, 161.0 ).xyxy;
	Pt *= Pt;
	vec4 V1xy_V2xy = mix( Pt.xyxy, Pt.zwzw, vec4( Pi_1.xy, Pi_2.xy ) );
	Pt = vec4( Pt.x, V1xy_V2xy.xz, Pt.z ) * vec4( Pt.y, V1xy_V2xy.yw, Pt.w );
	const vec3 SOMELARGEFLOATS = vec3( 635.298681, 682.357502, 668.926525 );
	const vec3 ZINC = vec3( 48.500388, 65.294118, 63.934599 );
	vec3 lowz_mods = vec3( 1.0 / ( SOMELARGEFLOATS.xyz + Pi.zzz * ZINC.xyz ) );
	vec3 highz_mods = vec3( 1.0 / ( SOMELARGEFLOATS.xyz + Pi_inc1.zzz * ZINC.xyz ) );
	Pi_1 = ( Pi_1.z < 0.5 ) ? lowz_mods : highz_mods;
	Pi_2 = ( Pi_2.z < 0.5 ) ? lowz_mods : highz_mods;
	vec4 hash_0 = fract( Pt * vec4( lowz_mods.x, Pi_1.x, Pi_2.x, highz_mods.x ) ) - 0.49999;
	vec4 hash_1 = fract( Pt * vec4( lowz_mods.y, Pi_1.y, Pi_2.y, highz_mods.y ) ) - 0.49999;
	vec4 hash_2 = fract( Pt * vec4( lowz_mods.z, Pi_1.z, Pi_2.z, highz_mods.z ) ) - 0.49999;
	vec4 norm = inversesqrt( hash_0 * hash_0 + hash_1 * hash_1 + hash_2 * hash_2 );
	hash_0 *= norm;
	hash_1 *= norm;
	hash_2 *= norm;
	vec4 grad_results = hash_0 * v1234_x + hash_1 * v1234_y + hash_2 * v1234_z;
	vec4 m = v1234_x * v1234_x + v1234_y * v1234_y + v1234_z * v1234_z;
	m = max(0.5 - m, 0.0);
	vec4 m2 = m*m;
	vec4 m3 = m*m2;
	vec4 temp = -6.0 * m2 * grad_results;
	float xderiv = dot( temp, v1234_x ) + dot( m3, hash_0 );
	float yderiv = dot( temp, v1234_y ) + dot( m3, hash_1 );
	float zderiv = dot( temp, v1234_z ) + dot( m3, hash_2 );
	return vec4( dot( m3, grad_results ), xderiv, yderiv, zderiv ) * 37.837227241611314102871574478976;
}
float _fbm(vec3 p, float octaves)
{
	const float lacunarity = 2.13628142, gain = 0.5;
	float freq = 1.0, amp = 1.0;
	float sum = 0.0, ampSum = 0.0;
	for(int i=0; i<int(octaves); i++) {
		sum += _noise(p*freq)*amp;
		ampSum += amp;
		freq *= lacunarity;
		amp *= gain;
	}
	return sum / ampSum;
}
float _fbm(vec2 p, float octaves)  { return _fbm(vec3(p, 0.0), octaves); }
float _fbm(float p, float octaves)   { return _fbm(vec3(p, 0.0, 0.0), octaves); }
float _turbulence(vec3 p, float octaves)
{
	const float lacunarity = 2.13628142, gain = 0.5;
	float sum = 0.0, ampSum = 0.0;
	float freq = 1.0, amp = 1.0;
	for(int i=0; i<int(octaves); i++) {
		sum += abs(_noise(p*freq))*amp;
		ampSum += amp;
		freq *= lacunarity;
		amp *= gain;
	}
	return sum / ampSum;
}
float _turbulence(vec2 p, float octaves)  { return _turbulence(vec3(p, 0.0), octaves); }
float _turbulence(float p, float octaves)   { return _turbulence(vec3(p, 0.0, 0.0), octaves); }
float _ridgedmf(vec3 p, float octaves)
{
	const float lacunarity = 2.13628142, gain = 0.5, offset = 1.0;
	float sum = 0.0, ampSum = 0.0;
	float freq = 1.0, amp = 0.5;
	float prev = 1.0;
	for(int i=0; i<int(octaves); i++) {
		float n = offset - abs(_noise(p*freq));
		n = n*n;
		sum += n*amp*prev;
		ampSum += amp;
		prev = n;
		freq *= lacunarity;
		amp *= gain;
	}
	return sum / ampSum;
}
float _ridgedmf(vec2 p, float octaves)  { return _ridgedmf(vec3(p, 0.0), octaves); }
float _ridgedmf(float p, float octaves)   { return _ridgedmf(vec3(p, 0.0, 0.0), octaves); }
void _voronoi(vec3 position, out float f1, out vec3 pos1, out float f2, out vec3 pos2, float jitter, int type)
{
	vec3 curCell = floor(position) + 0.5;
	f1 = f2 = 100000.0;
	float x, y, z;
	for(x = -1.0; x <= 1.0; x += 1.0)
	{
		for(y = -1.0; y <= 1.0; y += 1.0)
		{
			for(z = -1.0; z <= 1.0; z += 1.0)
			{
				vec3 testcell = curCell  + vec3(x,y,z);
				vec3 randomUVW = testcell * vec3(1.171, 1.073, 1.127);
				vec3 cellnoise = _perm(_perm(randomUVW.xy).r+randomUVW.z).xyz;
				vec3 pos = testcell + vec3(jitter)*(cellnoise - vec3(0.5));
				vec3 offset = pos - position;
				float dist;
				if (type == 0)
					dist = dot(offset, offset);
				else if (type == 1)
					dist = abs(offset.x)+abs(offset.y) + abs(offset.z);
				else
					dist = max(abs(offset.x), max(abs(offset.y), abs(offset.z)));
				if(dist < f1)
				{
					f2 = f1;
					pos2 = pos1;
					f1 = dist;
					pos1 = pos;
				}
				else if(dist < f2)
				{
					f2 = dist;
					pos2 = pos;
				}
			}
		}
	}
	if(type == 0)
	{
		f1 = sqrt(f1);
		f2 = sqrt(f2);
	}
}
void _voronoi(vec3 position, out float f1, out vec3 pos1, out float f2, out vec3 pos2)  { _voronoi(position, f1, pos1, f2, pos2, 0.9, 0); }
void _voronoi(vec2 position, out float f1, out vec2 pos1, out float f2, out vec2 pos2, float jitter, int type)
{
	vec2 curCell = floor(position) + 0.5;
	f1 = f2 = 100000.0;
	float x, y;
	for(x = -1.0; x <= 1.0; x += 1.0)
	{
		for(y = -1.0; y <= 1.0; y += 1.0)
		{
			vec2 testcell = curCell  + vec2(x,y);
			vec2 randomUVW = testcell * vec2(1.171, 1.073);
			vec2 cellnoise = _perm(randomUVW.xy).rg;
			vec2 pos = testcell + vec2(jitter)*(cellnoise - vec2(0.5));
			vec2 offset = pos - position;
			float dist;
			if (type == 0)
				dist = dot(offset, offset);
			else if (type == 1)
				dist = abs(offset.x)+abs(offset.y);
			else
				dist = max(abs(offset.x), abs(offset.y));
			if(dist < f1)
			{
				f2 = f1;
				pos2 = pos1;
				f1 = dist;
				pos1 = pos;
			}
			else if(dist < f2)
			{
				f2 = dist;
				pos2 = pos;
			}
		}
	}
	if(type == 0)
	{
		f1 = sqrt(f1);
		f2 = sqrt(f2);
	}
}
void _voronoi(vec2 position, out float f1, out vec2 pos1, out float f2, out vec2 pos2)  { _voronoi(position, f1, pos1, f2, pos2, 0.9, 0); }
// plane9 shaders pass the type selector as an integer literal; floatifyInts
// rewrites it to 0.0, which no longer matches the int type overload and the
// whole fragment fails to link (Around And Around rendered black). Accept a float
// selector and cast (WebGL2 has no implicit int/float promotion either way).
void _voronoi(vec3 position, out float f1, out vec3 pos1, out float f2, out vec3 pos2, float jitter, float type)  { _voronoi(position, f1, pos1, f2, pos2, jitter, int(type)); }
void _voronoi(vec2 position, out float f1, out vec2 pos1, out float f2, out vec2 pos2, float jitter, float type)  { _voronoi(position, f1, pos1, f2, pos2, jitter, int(type)); }
vec3 _blendScreen(vec3 src, vec3 dest)
{
	return (vec3(1.0) - ((vec3(1.0) - src) * (vec3(1.0) - dest)));
}
float _bump(float value, float start, float end, float width)
{
  return smoothstep(start-width,start,value)-smoothstep(end,end+width,value);
}
// By inigo quilez
float _cubicpulse(float value, float bumpCenter, float width)
{
    value = abs(value - bumpCenter);
    if (value>width) return 0.0;
    value /= width;
    return 1.0 - value*value*(3.0-2.0*value);
}
// By inigo quilez
float _parabola(float value, float sharpness)
{
    return pow(4.0*value*(1.0-value), sharpness);
}
// By inigo quilez
float _powercurve( float value, float a, float b )
{
    return pow( value, a ) * pow( 1.0-value, b );
}
// By inigo quilez
float _exppulse( float value, float k, float n )
{
    return exp( -k*pow(value,n) );
}
// By inigo quilez
float _impulse(float value, float width)
{
    float h = width*value;
    return h*exp(1.0-h);
}
vec3 _toneMappingFilmic(vec3 c)
{
   c = max(c-vec3(0.004),0.0);
   return (c*(6.2*c+0.5))/(c*(6.2*c+1.7)+0.06);
}
vec3 _uncharted2Tonemap(vec3 c)
{
	float A = 0.15, B = 0.50, C = 0.10, D = 0.20, E = 0.02, F = 0.30;
	return ((c * (c * A + B * C) + D * E) / (c * (c * A + B) + D * F)) - E / F;
}
vec3 _toneMappingUncharted2(vec3 c)
{
	return _uncharted2Tonemap(c) / _uncharted2Tonemap(vec3(11.2));
}
vec3 _tonemapACES(vec3 color)
{
    const float a = 2.51;
    const float b = 0.03;
    const float c = 2.43;
    const float d = 0.59;
    const float e = 0.14;
    return saturate((color * (a * color + b)) / (color * (c * color + d) + e));
}
vec3 _screenSpaceDither( vec2 screenPos )
{
	vec3 dither = vec3( dot( vec2( 171.0, 231.0 ), screenPos.xy));
    dither.rgb = fract( dither.rgb / vec3( 103.0, 71.0, 97.0 ) )-vec3(0.5);
    return dither.rgb / 255.0;
}
float _interleavedGradientNoise(vec2 p)
{
	vec3 magic = vec3(0.06711056, 0.00583715, 52.9829189);
	return fract(magic.z * fract(dot(p, magic.xy)));
}
vec3 _brightnessSaturationContrast(vec3 c, float brighness, float saturation, float contrast)
{
	vec3 brighnessCol = c * brighness;
	float luminance = dot(brighnessCol, vec3(0.2126, 0.7152, 0.0722));
	vec3 saturationCol = mix(vec3(luminance, luminance, luminance), brighnessCol, saturation);
	return mix(vec3(0.5), saturationCol, contrast);
}
vec3 _liftGammaGain(vec3 col, vec3 lift, vec3 gamma, vec3 gain)
{
	return pow((gain * (col + lift * (vec3(1.0) - col))),gamma);
}
// Shipped shaders pass vec4 uniforms (gIn1..3) as lift/gamma/gain; D3D
// sliced the .xyz implicitly, GLSL needs an explicit overload.
vec3 _liftGammaGain(vec3 col, vec4 lift, vec4 gamma, vec4 gain)
{
	return _liftGammaGain(col, lift.xyz, gamma.xyz, gain.xyz);
}
vec3 _palette(float t, vec3 brightness, vec3 contrast, vec3 a, vec3 b)
{
    return brightness + contrast*cos(PI2*(a*t+b));
}
vec3 _lightLambert(vec3 normal, vec3 lightDir, vec3 diffuseCol)
{
	return max(0.0, dot(lightDir, normal)) * diffuseCol;
}
// Plane9's Cook-Torrance helpers (Boxy). Verbatim from Point10's P9Glsl.js;
// the fresnel/roughness term and the directional light feed the metallic
// tinore material.
vec3 _fresnelRoughness(vec3 specularColor, float roughness, vec3 h, vec3 viewDir)
{
	return (specularColor + (max(vec3(1.0-(roughness*roughness)), specularColor) - specularColor) * pow((1.0 - saturate(dot(viewDir, h))), 5.0));
}
vec3 _shade(vec3 diffuseAlbedo,vec3 specularAlbedo, vec3 normal, float roughness, vec3 lightColor, vec3 lightDir, vec3 viewDir)
{
	vec3 h = normalize(viewDir + lightDir);
	float nDotH = saturate(dot(normal, h));
	float nDotL = saturate(dot(normal, lightDir));
	float nDotV = saturate(dot(normal, viewDir))+0.0001;
	float a = max(0.001, roughness * roughness);
	float a2 = a*a;
	float denominator = nDotH * nDotH * (a2 - 1.0) + 1.0;
	float d = a2 / (PI*denominator*denominator);
	float v1i = 1.0 / (nDotL + sqrt(a + (1.0 - a) * nDotL * nDotL));
	float v1o = 1.0 / (nDotV + sqrt(a + (1.0 - a) * nDotV * nDotV));
	float vis = v1i * v1o;
	float specular = d * vis;
	vec3 fresnel = specularAlbedo + (1.0 - specularAlbedo) * pow((1.0 - saturate(dot(lightDir, h))), 5.0);
	return (diffuseAlbedo * (1.0/PI) + specular * fresnel) * nDotL * lightColor;
}
vec3 _lightDirectional(vec3 diffuseAlbedo,vec3 specularAlbedo, vec3 normal, float roughness, vec3 lightColor, vec3 lightDir, vec3 viewDir)
{
	vec3 lighting = vec3(0.0);
	float nDotL = saturate(dot(normal, lightDir));
	if(nDotL > 0.0)
	{
		vec3 h = normalize(viewDir + lightDir);
		lighting = _shade(diffuseAlbedo,specularAlbedo, normal, roughness, lightColor, lightDir, viewDir);
	}
	return max(lighting, 0.0);
}
vec3 _blackBody(float temp)
{
	float temperature = temp / 100.0;
	vec3 col = vec3(255.0, 0.0, 0.0);
	if (temperature < 65.5) {
		col.g = temperature - 2.0;
		col.g = -155.254 - 0.446 * col.g + 104.49 * log(col.g);
		if (temperature > 20.0) {
			col.b = temperature - 10.0;
			col.b = -254.769 + 0.82 * col.b + 115.68 * log(col.b);
		}
	} else {
		col.r = temperature - 54.0;
		col.r = 352.97 + 0.06 * col.r - 39.8 * log(col.r);
		col.b = 255.0;
	}
	if (temperature > 65.0){	col.g = temperature - 49.5;		col.g = 325.45 + 0.079 * col.g - 28.08 * log(col.g);	}
	col = mix(vec3(0.0), col, smoothstep(900.0, 1000.0, temp));
	return pow(saturate(col/vec3(255.0)), vec3(2.2));	// toLinear
}
vec4 _cubic(float v)
{
    vec4 n = vec4(1.0, 2.0, 3.0, 4.0) - v;
    vec4 s = n * n * n;
    float x = s.x;
    float y = s.y - 4.0 * s.x;
    float z = s.z - 4.0 * s.y + 6.0 * s.x;
    float w = 6.0 - x - y - z;
    return vec4(x, y, z, w) * (1.0/6.0);
}
vec4 _textureBicubic(sampler2D tex, vec2 p, vec4 texSize)
{
    p = p * texSize.xy - vec2(0.5);
    vec2 fxy = fract(p);
    p -= fxy;
    vec4 xcubic = _cubic(fxy.x);
    vec4 ycubic = _cubic(fxy.y);
    vec4 c = vec4(p.x - 0.5, p.x + 1.5, p.y - 0.5, p.y + 1.5);
    vec4 s = vec4(xcubic.x + xcubic.y, xcubic.z + xcubic.w, ycubic.x + ycubic.y, ycubic.z + ycubic.w);
    vec4 offset = c + vec4(xcubic.y, xcubic.w, ycubic.y, ycubic.w) / s;
    offset *= texSize.zzww;
    vec4 sample0 = texture(tex, offset.xz);
    vec4 sample1 = texture(tex, offset.yz);
    vec4 sample2 = texture(tex, offset.xw);
    vec4 sample3 = texture(tex, offset.yw);
    float sx = s.x / (s.x + s.y);
    float sy = s.z / (s.z + s.w);
    return mix(mix(sample3, sample2, sx), mix(sample1, sample0, sx), sy);
}
vec4 _texturePanoramic(sampler2D tex, vec3 v)
{
	return texture(tex, vec2((1.0 + atan(v.x,-v.z) / PI)*0.5, 1.0-(acos(v.y) / PI)));
}
vec4 _texturePanoramicLod(sampler2D tex, vec3 v, float lod)
{
	return textureLod(tex, vec2((1.0 + atan(v.x,-v.z) / PI)*0.5, 1.0-(acos(v.y) / PI)), lod);
}
vec3 _fog(vec3 c, vec3 fogColor, float distToCam, float density)
{
	float fogAmount = 1.0 - exp( -distToCam*density );
	return mix(c, fogColor, fogAmount );
}
`;
