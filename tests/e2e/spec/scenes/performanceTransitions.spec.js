import { expect, test } from "@playwright/test";
import constants, {
  GROUP_ENABLED,
} from "../../../../src/renderer/src/application/constants";
import { modVApp } from "../../pageObjectModel";

const uniqueName = (prefix) => `${prefix} ${Date.now()}-${Math.random()}`;

const BROKEN_TRANSITION = `/*{
  "ISFVSN": "2",
  "INPUTS": [
    { "NAME": "startImage", "TYPE": "image" },
    { "NAME": "endImage", "TYPE": "image" },
    { "NAME": "progress", "TYPE": "float", "MIN": 0, "MAX": 1, "DEFAULT": 0 }
  ]
}*/
void main() {
  this shader does not compile;
}`;

async function createGroup(name, options = {}) {
  return modVApp.evaluateWorker(
    async ({ name, enabled, options }) => {
      const group = await self.store.dispatch("groups/createGroup", {
        ...options,
        name,
        enabled,
      });

      return group.id;
    },
    { name, enabled: GROUP_ENABLED, options },
  );
}

async function addModuleToGroup(groupId, moduleName) {
  return modVApp.evaluateWorker(
    async ({ groupId, moduleName }) => {
      const module = await self.store.dispatch("modules/makeActiveModule", {
        moduleName,
        moduleMeta: { enabled: true },
      });

      self.store.commit("groups/ADD_MODULE_TO_GROUP", {
        moduleId: module.$id,
        groupId,
      });

      return module.$id;
    },
    { groupId, moduleName },
  );
}

async function removeGroup(groupId) {
  await modVApp.evaluateWorker(
    async ({ groupId }) => {
      await self.store.dispatch("groups/removeGroup", { groupId });
    },
    { groupId },
  );
}

async function saveScene(name) {
  return modVApp.evaluateWorker(
    async ({ name }) =>
      self.store.dispatch("scenes/saveCurrentAsScene", { name }),
    { name },
  );
}

async function removeScene(sceneId) {
  await modVApp.evaluateWorker(
    async ({ sceneId }) => {
      await self.store.dispatch("scenes/removeScene", { sceneId });
    },
    { sceneId },
  );
}

async function ensureUserGroup() {
  await modVApp.evaluateWorker(
    async ({ galleryGroupName, enabled }) => {
      const userGroups = self.store.state.groups.groups.filter(
        (group) => group.name !== galleryGroupName,
      );

      if (!userGroups.length) {
        await self.store.dispatch("groups/createGroup", { enabled });
      }
    },
    {
      galleryGroupName: constants.GALLERY_GROUP_NAME,
      enabled: GROUP_ENABLED,
    },
  );
}

async function waitForMainOutput() {
  // The app transfers the output canvas to the worker after setupMedia, which
  // can take a while in the headless e2e environment. Install a deterministic
  // 2D output canvas if one is not available yet so the render loop has
  // somewhere to composite to.
  const hasMain = await modVApp.evaluateWorker(
    () => !!self.store.state.outputs.main,
  );

  if (hasMain) {
    return;
  }

  await modVApp.evaluateWorker(async () => {
    if (self.store.state.outputs.main) {
      return;
    }

    const canvas = new OffscreenCanvas(300, 300);

    await self.store.dispatch("outputs/setMainOutput", canvas.getContext("2d"));
  });
}

async function isolateScenes(sceneIds) {
  return modVApp.evaluateWorker(
    async ({ sceneIds }) => {
      const previous = self.store.state.scenes.scenes.map((scene) => ({
        id: scene.id,
        enabled: scene.enabled !== false,
      }));

      for (let i = 0; i < previous.length; i++) {
        self.store.commit("scenes/UPDATE_SCENE", {
          sceneId: previous[i].id,
          data: { enabled: sceneIds.includes(previous[i].id) },
        });
      }

      return previous;
    },
    { sceneIds },
  );
}

async function restoreScenes(previous) {
  await modVApp.evaluateWorker(
    async ({ previous }) => {
      for (let i = 0; i < previous.length; i++) {
        self.store.commit("scenes/UPDATE_SCENE", {
          sceneId: previous[i].id,
          data: { enabled: previous[i].enabled },
        });
      }
    },
    { previous },
  );
}

async function tick(iterations, delta = 100) {
  await modVApp.evaluateWorker(
    async ({ iterations, delta }) => {
      for (let i = 0; i < iterations; i++) {
        await self.store.dispatch("scenes/tick", { delta });
      }
    },
    { iterations, delta },
  );
}

async function createVisualScenes(prefix) {
  await ensureUserGroup();

  const groupA = await createGroup(`${prefix} A Group`);
  await addModuleToGroup(groupA, "Counter");
  const sceneA = await saveScene(`${prefix} A`);
  await removeGroup(groupA);

  const groupB = await createGroup(`${prefix} B Group`, { inherit: false });
  await addModuleToGroup(groupB, "Bar");
  const sceneB = await saveScene(`${prefix} B`);
  await removeGroup(groupB);

  return { sceneA, sceneB };
}

async function findIsfTransitionName() {
  return modVApp.evaluateWorker(() => {
    const registered = Object.values(self.store.state.modules.registered);
    const candidates = registered
      .filter(
        (module) =>
          module.props &&
          module.props.startImage &&
          module.props.endImage &&
          module.props.progress,
      )
      .map((module) => module.meta.name);

    return candidates.includes("CrossZoom") ? "CrossZoom" : candidates[0];
  });
}

async function beginTransition(transitionName) {
  return modVApp.evaluateWorker(
    async ({ transitionName }) => {
      const sample = () => {
        const main = self.store.state.outputs.main;

        if (!main) {
          return { nonBlank: 0, hash: 0, width: 0, height: 0 };
        }

        const { width, height } = main.canvas;
        const data = main.getImageData(0, 0, width, height).data;
        let nonBlank = 0;
        let hash = 2166136261;

        // Sample every fourth pixel to keep this cheap enough to run inside a
        // frame.
        for (let i = 0; i < data.length; i += 16) {
          if (
            data[i + 3] > 8 &&
            (data[i] > 8 || data[i + 1] > 8 || data[i + 2] > 8)
          ) {
            nonBlank++;
          }

          hash ^= data[i] | (data[i + 1] << 8) | (data[i + 2] << 16);
          hash = Math.imul(hash, 16777619) >>> 0;
        }

        return { nonBlank, hash, width, height };
      };

      const pickTransition = () => ({
        name: self.store.getters["scenes/currentTransitionName"],
        progress: self.store.getters["scenes/transitionProgress"],
        transitioning: self.store.state.scenes.transitioning,
      });

      const nextFrame = () =>
        new Promise((resolve) => requestAnimationFrame(resolve));

      // Wait for the outgoing Scene to be composited so the comparison below
      // is against a real frame rather than an empty canvas.
      let before = sample();

      for (let i = 0; i < 120 && before.nonBlank === 0; i++) {
        await nextFrame();
        before = sample();
      }

      await self.store.dispatch("scenes/updatePerformance", {
        enabled: false,
        allowedTransitions: [transitionName],
      });
      await self.store.dispatch("scenes/nextScene");

      const started = pickTransition();
      const history = [];

      for (let i = 0; i < 60; i++) {
        await nextFrame();

        const transition = pickTransition();
        const frame = sample();

        history.push({
          ...transition,
          nonBlank: frame.nonBlank,
          hash: frame.hash,
        });

        if (!transition.transitioning) {
          break;
        }
      }

      // If the render loop is throttled, finish the transition deterministically.
      if (self.store.state.scenes.transitioning) {
        for (let i = 0; i < 60; i++) {
          await self.store.dispatch("scenes/tick", { delta: 100 });
        }

        history.push({
          ...pickTransition(),
          nonBlank: sample().nonBlank,
          hash: sample().hash,
        });
      }

      const during =
        history.find(
          (frame) =>
            frame.transitioning &&
            frame.name === transitionName &&
            frame.nonBlank > 0 &&
            frame.hash !== before.hash,
        ) ||
        history.find(
          (frame) =>
            frame.transitioning &&
            frame.nonBlank > 0 &&
            frame.hash !== before.hash,
        ) ||
        null;

      return {
        before,
        started,
        during,
        history,
        final: pickTransition(),
      };
    },
    { transitionName },
  );
}

async function completeTransition() {
  // Six seconds of manual ticks at 100ms each is longer than any transition
  // duration used by these tests.
  await tick(60);

  return modVApp.evaluateWorker(() => ({
    transitioning: self.store.state.scenes.transitioning,
    name: self.store.getters["scenes/currentTransitionName"],
    groups: self.store.state.groups.groups
      .filter((group) => !group.name.includes("Gallery"))
      .map((group) => group.name),
  }));
}

async function cleanup({ sceneA, sceneB, previousScenes, brokenName }) {
  await modVApp.evaluateWorker(async () => {
    await self.store.dispatch("scenes/stopPerformance");
  });

  if (brokenName) {
    await modVApp.evaluateWorker(
      async ({ brokenName }) => {
        self.store.commit("modules/REMOVE_REGISTERED_MODULE", {
          moduleName: brokenName,
        });
      },
      { brokenName },
    );
  }

  if (previousScenes) {
    await restoreScenes(previousScenes);
  }

  if (sceneA) {
    await removeScene(sceneA.id);
  }

  if (sceneB) {
    await removeScene(sceneB.id);
  }

  await ensureUserGroup();
}

async function runShaderTransitionTest(
  transitionName,
  expectedName = transitionName,
) {
  await waitForMainOutput();

  const prefix = uniqueName(`E2E ${transitionName}`);
  const { sceneA, sceneB } = await createVisualScenes(prefix);
  const previousScenes = await isolateScenes([sceneA.id, sceneB.id]);

  try {
    await modVApp.evaluateWorker(
      async ({ sceneAId }) => {
        await self.store.dispatch("scenes/updatePerformance", {
          enabled: false,
          allowedTransitions: ["None"],
          transitionSeconds: 0.5,
          minSeconds: 20,
          maxSeconds: 30,
          order: "in-order",
        });

        await self.store.dispatch("scenes/applyScene", { sceneId: sceneAId });
        await self.store.dispatch("scenes/startPerformance");
      },
      { sceneAId: sceneA.id },
    );

    const result = await beginTransition(transitionName);

    expect(result.started.name).toBe(expectedName);
    expect(result.started.transitioning).toBe(true);
    expect(
      result.history.some((frame) => frame.progress > result.started.progress),
    ).toBe(true);

    expect(result.during).toBeTruthy();
    expect(result.during.nonBlank).toBeGreaterThan(0);
    expect(result.during.hash).not.toBe(result.before.hash);

    const completed = await completeTransition();

    expect(completed.transitioning).toBe(false);
    expect(completed.name).toBe(null);
    expect(completed.groups).not.toContain(`${prefix} A Group`);
    expect(completed.groups).toContain(`${prefix} B Group`);
  } finally {
    await cleanup({ sceneA, sceneB, previousScenes });
  }
}

test("a native shader transition renders and completes in performance mode", async () => {
  await runShaderTransitionTest("Three Wipe");
});

test("an unknown transition falls back to Fade", async () => {
  await runShaderTransitionTest(`E2E Missing Transition ${Date.now()}`, "Fade");
});

test("an ISF transition renders and completes in performance mode", async () => {
  const transitionName = await findIsfTransitionName();

  expect(transitionName).toBeTruthy();

  await runShaderTransitionTest(transitionName);
});

test("an uncompilable transition falls back to Fade", async () => {
  await waitForMainOutput();

  const prefix = uniqueName("E2E Broken Transition");
  const brokenName = uniqueName("E2E Broken Transition Shader");

  await modVApp.evaluateWorker(
    async ({ brokenName, fragmentShader }) => {
      await self.store.dispatch("modules/registerModule", {
        module: {
          meta: {
            name: brokenName,
            author: "e2e",
            version: "1.0.0",
            type: "isf",
          },
          fragmentShader,
        },
      });
    },
    { brokenName, fragmentShader: BROKEN_TRANSITION },
  );

  const { sceneA, sceneB } = await createVisualScenes(prefix);
  const previousScenes = await isolateScenes([sceneA.id, sceneB.id]);

  try {
    await modVApp.evaluateWorker(
      async ({ sceneAId }) => {
        await self.store.dispatch("scenes/updatePerformance", {
          enabled: false,
          allowedTransitions: ["None"],
          transitionSeconds: 0.5,
          minSeconds: 20,
          maxSeconds: 30,
          order: "in-order",
        });

        await self.store.dispatch("scenes/applyScene", { sceneId: sceneAId });
        await self.store.dispatch("scenes/startPerformance");
      },
      { sceneAId: sceneA.id },
    );

    const result = await beginTransition(brokenName);

    expect(result.started.transitioning).toBe(true);
    expect(result.history.some((frame) => frame.name === "Fade")).toBe(true);
    expect(result.during).toBeTruthy();
    expect(result.during.nonBlank).toBeGreaterThan(0);
    expect(result.during.hash).not.toBe(result.before.hash);

    const completed = await completeTransition();

    expect(completed.transitioning).toBe(false);
    expect(completed.groups).not.toContain(`${prefix} A Group`);
    expect(completed.groups).toContain(`${prefix} B Group`);
  } finally {
    await cleanup({
      sceneA,
      sceneB,
      previousScenes,
      brokenName,
    });
  }
});
