import { expect, test } from "@playwright/test";
import constants, {
  GROUP_ENABLED,
} from "../../../../src/renderer/src/application/constants";
import { modVApp } from "../../pageObjectModel";

const uniqueName = (prefix) => `${prefix} ${Date.now()}-${Math.random()}`;

async function createGroup(name) {
  return modVApp.evaluateWorker(
    async ({ name, enabled }) => {
      const group = await self.store.dispatch("groups/createGroup", {
        name,
        enabled,
      });

      return group.id;
    },
    { name, enabled: GROUP_ENABLED },
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

async function createRowScene(prefix, rowName, moduleName, timing) {
  const groupId = await createGroup(rowName);
  await addModuleToGroup(groupId, moduleName);

  const scene = await saveScene(`${prefix} ${rowName}`);

  await removeGroup(groupId);

  await modVApp.evaluateWorker(
    async ({ sceneId, timing }) => {
      self.store.commit("scenes/UPDATE_SCENE", {
        sceneId,
        data: {
          timingOverride: true,
          minSeconds: timing.minSeconds,
          maxSeconds: timing.maxSeconds,
        },
      });
    },
    { sceneId: scene.id, timing },
  );

  return scene;
}

async function removeUserGroups() {
  await modVApp.evaluateWorker(
    async ({ galleryGroupName }) => {
      await self.store.dispatch("scenes/stopPerformance");

      const userGroups = self.store.state.groups.groups.filter(
        (group) => group.name !== galleryGroupName,
      );

      for (let i = 0; i < userGroups.length; i++) {
        await self.store.dispatch("groups/removeGroup", {
          groupId: userGroups[i].id,
        });
      }

      const groupModuleIds = new Set(
        self.store.state.groups.groups.flatMap((group) => group.modules),
      );

      for (const module of Object.values(self.store.state.modules.active)) {
        if (!groupModuleIds.has(module.$id) && !module.meta.isGallery) {
          await self.store.dispatch("modules/removeActiveModule", {
            moduleId: module.$id,
          });
        }
      }
    },
    { galleryGroupName: constants.GALLERY_GROUP_NAME },
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

async function configurePerformance({ rowNames, transitionSeconds = 2 }) {
  await modVApp.evaluateWorker(
    async ({ rowNames, transitionSeconds }) => {
      await self.store.dispatch("scenes/updatePerformance", {
        enabled: false,
        allowedTransitions: ["None"],
        transitionSeconds,
        order: "in-order",
        rowNames,
        minSeconds: 8,
        maxSeconds: 24,
      });
    },
    { rowNames, transitionSeconds },
  );
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
  if (!previous) {
    return;
  }

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

async function getPerformanceState() {
  return modVApp.evaluateWorker(
    async ({ galleryGroupName }) => {
      const activeRows = self.store.getters["scenes/activeRows"];
      const groups = self.store.state.groups.groups.filter(
        (group) =>
          group.name !== galleryGroupName && group.transitionGhost !== true,
      );

      return {
        activeRows: JSON.parse(JSON.stringify(activeRows)),
        rowTransitions: {
          0: !!self.store.getters["scenes/rowTransition"](0),
          1: !!self.store.getters["scenes/rowTransition"](1),
        },
        groupNames: groups.map((group) => group.name),
      };
    },
    { galleryGroupName: constants.GALLERY_GROUP_NAME },
  );
}

async function cleanup({ sceneIds, previousScenes }) {
  await modVApp.evaluateWorker(async () => {
    await self.store.dispatch("scenes/stopPerformance");
  });

  await restoreScenes(previousScenes);

  for (let i = 0; i < sceneIds.length; i++) {
    await removeScene(sceneIds[i]);
  }

  await configurePerformance({ rowNames: [] });
  await removeUserGroups();
  await ensureUserGroup();
}

test("cells with different timing advance independently and only the advancing row fades", async () => {
  const prefix = uniqueName("E2E Cell Timing");

  await removeUserGroups();
  await configurePerformance({ rowNames: ["fast", "slow"] });

  const fastA = await createRowScene(prefix, "fast", "Counter", {
    minSeconds: 1,
    maxSeconds: 1,
  });
  const fastB = await createRowScene(prefix, "fast", "Bar", {
    minSeconds: 1,
    maxSeconds: 1,
  });
  const slowA = await createRowScene(prefix, "slow", "Counter", {
    minSeconds: 10,
    maxSeconds: 10,
  });
  const slowB = await createRowScene(prefix, "slow", "Bar", {
    minSeconds: 10,
    maxSeconds: 10,
  });

  const sceneIds = [fastA.id, fastB.id, slowA.id, slowB.id];
  const previousScenes = await isolateScenes(sceneIds);

  try {
    // Pin the per-cell random roll to 0 so the fast row's target is exactly
    // its one second minimum and the slow row's is exactly ten seconds.
    // normalizeSceneTiming raises maxSeconds to min+1, so without this the
    // fast row could choose a target up to two seconds.
    await modVApp.evaluateWorker(async () => {
      const originalRandom = Math.random;

      Math.random = () => 0;

      try {
        await self.store.dispatch("scenes/startPerformance");

        // Twelve ticks is long enough for the fast row to advance and start
        // its fade, but nowhere near the slow row's deadline.
        for (let i = 0; i < 12; i++) {
          await self.store.dispatch("scenes/tick", { delta: 100 });
        }
      } finally {
        Math.random = originalRandom;
      }
    });

    const state = await getPerformanceState();

    expect(state.activeRows[0].sceneId).toBe(fastB.id);
    expect(state.activeRows[1].sceneId).toBe(slowA.id);
    expect(state.rowTransitions[0]).toBe(true);
    expect(state.rowTransitions[1]).toBe(false);
    expect(state.groupNames).toContain("slow");
  } finally {
    await cleanup({ sceneIds, previousScenes });
  }
});

test("max cell time stays the upper bound of a cell's random dwell", async () => {
  const prefix = uniqueName("E2E Cell Max");

  await removeUserGroups();
  await configurePerformance({ rowNames: ["cell"] });

  const first = await createRowScene(prefix, "cell", "Counter", {
    minSeconds: 2,
    maxSeconds: 3,
  });
  const second = await createRowScene(prefix, "cell", "Bar", {
    minSeconds: 2,
    maxSeconds: 3,
  });

  const sceneIds = [first.id, second.id];
  const previousScenes = await isolateScenes(sceneIds);

  try {
    await modVApp.evaluateWorker(async () => {
      await self.store.dispatch("scenes/startPerformance");
    });

    // A target in [2, 3] cannot force an advance before two seconds.
    await tick(19);

    let state = await getPerformanceState();

    expect(state.activeRows[0].sceneId).toBe(first.id);

    // Past three seconds the upper bound must have forced the change.
    await tick(12);

    state = await getPerformanceState();

    expect(state.activeRows[0].sceneId).toBe(second.id);
  } finally {
    await cleanup({ sceneIds, previousScenes });
  }
});
