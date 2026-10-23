/**
 * Shared GPU resource cleanup for renderers.
 *
 * Scene transitions rebuild every Module with a fresh id, so anything a
 * renderer allocates per Module has to be released when the old Scene's Groups
 * are removed. Without this, performance mode leaks shader programs, textures
 * and framebuffers on every transition until the GPU gives up.
 */

export function getModuleId(moduleOrId) {
  if (!moduleOrId) {
    return null;
  }

  if (typeof moduleOrId === "string") {
    return moduleOrId;
  }

  return moduleOrId.$id || null;
}

export function disposeISFRenderer(renderer) {
  if (!renderer) {
    return;
  }

  // `cleanup` releases the ISF render buffers. Fall back to destroying them
  // directly so a partially constructed renderer is still cleaned up.
  if (typeof renderer.cleanup === "function") {
    renderer.cleanup();
  } else if (Array.isArray(renderer.renderBuffers)) {
    for (let i = 0, len = renderer.renderBuffers.length; i < len; i++) {
      const buffer = renderer.renderBuffers[i];

      if (buffer && typeof buffer.destroy === "function") {
        buffer.destroy();
      }
    }
  }

  // ISFRenderer.cleanup does not release the shader programs or the textures
  // created for image inputs, so do that here.
  if (renderer.program && typeof renderer.program.cleanup === "function") {
    renderer.program.cleanup();
  }

  if (
    renderer.paintProgram &&
    typeof renderer.paintProgram.cleanup === "function"
  ) {
    renderer.paintProgram.cleanup();
  }

  if (renderer.uniforms) {
    const uniforms = Object.values(renderer.uniforms);

    for (let i = 0, len = uniforms.length; i < len; i++) {
      const uniform = uniforms[i];

      if (uniform && uniform.texture && uniform.texture.destroy) {
        uniform.texture.destroy();
      }
    }
  }
}

export function disposePexCommand(pex, command) {
  if (!pex || !command) {
    return;
  }

  if (command.pipeline) {
    pex.dispose(command.pipeline);
    command.pipeline = null;
  }
}

export function createBoundedCache(limit = 4, dispose = () => {}) {
  const maxEntries = Math.max(1, limit);
  const entries = new Map();

  return {
    get size() {
      return entries.size;
    },

    has(name) {
      return entries.has(name);
    },

    get(name) {
      if (!entries.has(name)) {
        return null;
      }

      const value = entries.get(name);
      // Refresh recency so frequently used entries are kept.
      entries.delete(name);
      entries.set(name, value);

      return value;
    },

    set(name, value) {
      if (entries.has(name)) {
        entries.delete(name);
      }

      entries.set(name, value);

      while (entries.size > maxEntries) {
        const oldest = entries.entries().next().value;

        if (!oldest) {
          break;
        }

        const [oldName, oldValue] = oldest;

        entries.delete(oldName);
        dispose(oldValue, oldName);
      }

      return value;
    },

    clear() {
      for (const value of entries.values()) {
        dispose(value);
      }

      entries.clear();
    },
  };
}
