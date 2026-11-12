/**
 * Media-library mirroring for engine mode (WS-10 dependency).
 *
 * Engine mode has no worker: the renderer store is a mirror of the engine, and
 * the main process' media-manager IPC used to dispatch only into the engine
 * facade. The UI reads the library (`media.media[currentProject].video`) and
 * `media.path` from the mirror, so the replay below is what makes a video file
 * selectable — and its absolute path resolvable for `media/start-file` — in
 * engine mode.
 */

import { describe, expect, it } from "vitest";

import {
  applyMediaPathToMirror,
  applyMediaStateToMirror,
  applyMediaUpdateToMirror,
} from "../../src/renderer/src/application/media-mirror.js";

function recordingStore() {
  const commits = [];

  return {
    commits,
    commit(type, payload) {
      commits.push({ type, payload });
    },
  };
}

describe("applyMediaStateToMirror", () => {
  const state = {
    path: "/home/u/.config/VmodV/media",
    media: {
      default: {
        video: {
          clip: { name: "clip", path: "default/video/clip.mp4" },
        },
        image: {
          still: { name: "still", path: "default/image/still.png" },
        },
      },
    },
  };

  it("clears, sets the media directory and re-adds every item", () => {
    const store = recordingStore();

    applyMediaStateToMirror(store, state);

    expect(store.commits.map((commit) => commit.type)).toEqual([
      "media/CLEAR_MEDIA_STATE",
      "media/SET_MEDIA_DIRECTORY_PATH",
      "media/ADD",
      "media/ADD",
    ]);
    expect(store.commits[1].payload).toEqual({
      path: "/home/u/.config/VmodV/media",
    });
    expect(store.commits[2].payload).toEqual({
      project: "default",
      folder: "video",
      item: { name: "clip", path: "default/video/clip.mp4" },
    });
  });

  it("ignores a missing or malformed payload", () => {
    const store = recordingStore();

    applyMediaStateToMirror(store, undefined);
    applyMediaStateToMirror(store, null);
    applyMediaStateToMirror(store, "nope");

    expect(store.commits).toEqual([]);
  });

  it("tolerates a state without a path or projects", () => {
    const store = recordingStore();

    applyMediaStateToMirror(store, { media: {} });

    expect(store.commits).toEqual([
      { type: "media/CLEAR_MEDIA_STATE", payload: undefined },
    ]);
  });
});

describe("applyMediaUpdateToMirror", () => {
  it("adds the item the main process just indexed", () => {
    const store = recordingStore();

    applyMediaUpdateToMirror(store, {
      type: "media/ADD",
      payload: {
        project: "default",
        folder: "video",
        item: { name: "clip", path: "default/video/clip.mp4" },
      },
    });

    expect(store.commits).toEqual([
      {
        type: "media/ADD",
        payload: {
          project: "default",
          folder: "video",
          item: { name: "clip", path: "default/video/clip.mp4" },
        },
      },
    ]);
  });

  it("ignores incomplete updates", () => {
    const store = recordingStore();

    applyMediaUpdateToMirror(store, { payload: { project: "default" } });
    applyMediaUpdateToMirror(store, {});

    expect(store.commits).toEqual([]);
  });
});

describe("applyMediaPathToMirror", () => {
  it("sets the media directory path", () => {
    const store = recordingStore();

    applyMediaPathToMirror(store, { path: "/media" });

    expect(store.commits).toEqual([
      { type: "media/SET_MEDIA_DIRECTORY_PATH", payload: { path: "/media" } },
    ]);
  });

  it("ignores an empty path", () => {
    const store = recordingStore();

    applyMediaPathToMirror(store, { path: "" });
    applyMediaPathToMirror(store, undefined);

    expect(store.commits).toEqual([]);
  });
});
