import ISFTexture from "interactive-shader-format/src/ISFTexture.js";

let installed = false;

/**
 * The vendored ISF renderer allocates PERSISTENT FLOAT buffers with an
 * unsized RGBA internal format, which WebGL2 rejects. Several ported
 * generators (PixelTunnel, Fractal Mage, LOPYFrac, Fractal Mountain) store
 * accumulated time in those buffers, so without this they render black or
 * freeze after the first frame.
 */
export function installFloatBufferSupport(gl) {
  const supported = !!gl.getExtension("EXT_color_buffer_float");

  if (installed) {
    return;
  }

  installed = true;

  if (!supported) {
    return;
  }

  const originalSetSize = ISFTexture.prototype.setSize;

  ISFTexture.prototype.setSize = function setSize(w, h) {
    if (!this.float) {
      return originalSetSize.call(this, w, h);
    }

    if (this.width === w && this.height === h) {
      return;
    }

    this.width = w;
    this.height = h;

    this.gl.bindTexture(this.gl.TEXTURE_2D, this.texture);
    // Float textures are not filterable without OES_texture_float_linear;
    // PERSISTENT buffers are sampled at exact texel centres anyway.
    this.gl.texParameteri(
      this.gl.TEXTURE_2D,
      this.gl.TEXTURE_MIN_FILTER,
      this.gl.NEAREST,
    );
    this.gl.texParameteri(
      this.gl.TEXTURE_2D,
      this.gl.TEXTURE_MAG_FILTER,
      this.gl.NEAREST,
    );
    this.gl.texImage2D(
      this.gl.TEXTURE_2D,
      0,
      this.gl.RGBA32F,
      w,
      h,
      0,
      this.gl.RGBA,
      this.gl.FLOAT,
      null,
    );
  };
}
