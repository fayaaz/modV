// Small LRU cache for warmed Point10 Scene instances. A warmed instance costs
// GPU memory (the fire/smoke scenes allocate hundreds of megabytes of 3D
// textures), so the cache is deliberately tiny and evicts the least recently
// used Scene when a new one is warmed.
let sharedCache = null;

/**
 * Stores own the process-wide warm cache so worker code (which must not import
 * the renderer-facing adapter) can drop it when performance mode stops.
 */
export function registerWarmCache(cache) {
  sharedCache = cache;
}

export function clearWarmInstances() {
  if (sharedCache) {
    sharedCache.clear();
  }
}

export function createWarmCache({
  limit = 1,
  onEvict = null,
  onClear = null,
} = {}) {
  const entries = new Map();

  const evict = (value) => {
    if (typeof onEvict === "function") {
      onEvict(value);
    }
  };

  return {
    get size() {
      return entries.size;
    },

    has(key) {
      return entries.has(key);
    },

    take(key) {
      if (!entries.has(key)) {
        return null;
      }

      const value = entries.get(key);
      entries.delete(key);

      return value;
    },

    set(key, value) {
      // Re-setting the same key refreshes its position in the LRU order.
      if (entries.has(key)) {
        entries.delete(key);
      }

      entries.set(key, value);

      while (entries.size > limit) {
        const oldestKey = entries.keys().next().value;
        const oldest = entries.get(oldestKey);

        entries.delete(oldestKey);
        evict(oldest);
      }

      return value;
    },

    delete(key) {
      if (!entries.has(key)) {
        return null;
      }

      const value = entries.get(key);
      entries.delete(key);

      return value;
    },

    values() {
      return Array.from(entries.values());
    },

    clear() {
      const values = Array.from(entries.values());
      entries.clear();

      for (let i = 0; i < values.length; i++) {
        evict(values[i]);
      }

      if (typeof onClear === "function") {
        onClear();
      }
    },
  };
}
