import { describe, expect, it, vi } from "vitest";
import { disposeISFRenderer } from "../../src/renderer/src/application/renderers/disposal.js";

function fakeGl() {
  return {
    deleteShader: vi.fn(),
    deleteProgram: vi.fn(),
    deleteBuffer: vi.fn(),
  };
}

function fakeProgram(gl, extra = {}) {
  return {
    gl,
    fShader: { id: "fs" },
    vShader: { id: "vs" },
    program: { id: "p" },
    cleanup: vi.fn(),
    ...extra,
  };
}

function fakeRenderer({ cleanup = true, gl } = {}) {
  const programGl = gl === undefined ? fakeGl() : gl;
  const renderer = {
    program: fakeProgram(programGl),
    paintProgram: fakeProgram(programGl),
    uniforms: {
      image: { texture: { destroy: vi.fn() } },
      plain: { value: 1 },
    },
    renderBuffers: [],
  };

  if (cleanup) {
    renderer.cleanup = vi.fn();
  }

  return { renderer, gl: programGl };
}

describe("disposeISFRenderer", () => {
  it("disposes nothing when there is no renderer", () => {
    expect(() => disposeISFRenderer(null)).not.toThrow();
    expect(() => disposeISFRenderer(undefined)).not.toThrow();
  });

  it("calls the renderer cleanup for the render buffers", () => {
    const { renderer } = fakeRenderer();

    disposeISFRenderer(renderer);

    expect(renderer.cleanup).toHaveBeenCalledTimes(1);
  });

  it("destroys render buffers directly when cleanup is unavailable", () => {
    const { renderer } = fakeRenderer({ cleanup: false });
    const buffer = { destroy: vi.fn() };

    renderer.renderBuffers = [buffer, null];

    disposeISFRenderer(renderer);

    expect(buffer.destroy).toHaveBeenCalledTimes(1);
  });

  it("releases the shaders and programs of both programs", () => {
    const { renderer, gl } = fakeRenderer();

    disposeISFRenderer(renderer);

    expect(gl.deleteShader).toHaveBeenCalledTimes(4);
    expect(gl.deleteShader).toHaveBeenCalledWith(renderer.program.fShader);
    expect(gl.deleteShader).toHaveBeenCalledWith(renderer.program.vShader);
    expect(gl.deleteShader).toHaveBeenCalledWith(renderer.paintProgram.fShader);
    expect(gl.deleteShader).toHaveBeenCalledWith(renderer.paintProgram.vShader);
    expect(gl.deleteProgram).toHaveBeenCalledTimes(2);
    expect(gl.deleteProgram).toHaveBeenCalledWith(renderer.program.program);
    expect(gl.deleteProgram).toHaveBeenCalledWith(
      renderer.paintProgram.program,
    );
  });

  it("never deletes the vertex buffers of the shared ISF context", () => {
    const { renderer, gl } = fakeRenderer();

    disposeISFRenderer(renderer);

    expect(gl.deleteBuffer).not.toHaveBeenCalled();
    // The GL programs are released without the whole-program cleanup path.
    expect(renderer.program.cleanup).not.toHaveBeenCalled();
    expect(renderer.paintProgram.cleanup).not.toHaveBeenCalled();
  });

  it("falls back to the program cleanup when no GL context is available", () => {
    const { renderer } = fakeRenderer({ gl: null });

    disposeISFRenderer(renderer);

    expect(renderer.program.cleanup).toHaveBeenCalledTimes(1);
    expect(renderer.paintProgram.cleanup).toHaveBeenCalledTimes(1);
  });

  it("destroys textures created for image inputs", () => {
    const { renderer } = fakeRenderer();

    disposeISFRenderer(renderer);

    expect(renderer.uniforms.image.texture.destroy).toHaveBeenCalledTimes(1);
  });
});
