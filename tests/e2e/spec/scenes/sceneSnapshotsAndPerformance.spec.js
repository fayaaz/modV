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

test("a saved Scene restores its Groups and Modules", async () => {
  const name = uniqueName("E2E Snapshot");
  const groupId = await createGroup(name);
  const moduleId = await addModuleToGroup(groupId, "Ball");

  await modVApp.evaluateWorker(async () => {
    await self.store.dispatch("scenes/updateAutopilot", {
      transition: "None",
    });
  });

  expect(
    await modVApp.evaluateWorkerState("scenes.autopilot.allowedTransitions"),
  ).toEqual(["None"]);

  const scene = await saveScene(name);

  expect(scene.name).toBe(name);

  const savedGroup = scene.groups.find((group) => group.name === name);

  expect(savedGroup).toBeTruthy();
  expect(savedGroup.modules).toContain(moduleId);
  expect(scene.modules[moduleId].$moduleName).toBe("Ball");

  // Remove the source Group so a successful Apply has to rebuild it.
  await modVApp.evaluateWorker(
    async ({ groupId }) => {
      await self.store.dispatch("groups/removeGroup", { groupId });
    },
    { groupId },
  );

  await modVApp.evaluateWorker(
    async ({ sceneId }) => {
      await self.store.dispatch("scenes/applyScene", { sceneId });
    },
    { sceneId: scene.id },
  );

  const groups = await modVApp.evaluateWorkerState("groups.groups");
  const restoredGroup = groups.find((group) => group.name === name);

  expect(restoredGroup).toBeTruthy();
  expect(restoredGroup.modules).toHaveLength(1);

  const activeModules = await modVApp.evaluateWorkerState("modules.active");
  expect(activeModules[restoredGroup.modules[0]].$moduleName).toBe("Ball");

  await removeScene(scene.id);
  await ensureUserGroup();
});

test("performance mode starts, advances and stops", async () => {
  const name = uniqueName("E2E Performance");

  await ensureUserGroup();
  await modVApp.evaluateWorker(async () => {
    await self.store.dispatch("scenes/updateAutopilot", {
      allowedTransitions: ["Fade"],
      order: "in-order",
      transitionSeconds: 3,
      minSeconds: 1,
      maxSeconds: 2,
    });
  });

  expect(
    await modVApp.evaluateWorkerState("scenes.autopilot.allowedTransitions"),
  ).toEqual(["Fade"]);

  const sceneA = await saveScene(`${name} A`);
  const sceneB = await saveScene(`${name} B`);

  const started = await modVApp.evaluateWorker(async () => {
    await self.store.dispatch("scenes/startPerformance");

    return {
      enabled: self.store.state.scenes.autopilot.enabled,
      currentSceneId: self.store.state.scenes.autopilot.currentSceneId,
    };
  });

  expect(started.enabled).toBe(true);
  expect(started.currentSceneId).toBeTruthy();

  await modVApp.evaluateWorker(async () => {
    await self.store.dispatch("scenes/nextScene");
  });

  const nextSceneId = await modVApp.evaluateWorkerState(
    "scenes.autopilot.currentSceneId",
  );

  expect(nextSceneId).toBeTruthy();
  expect(nextSceneId).not.toBe(started.currentSceneId);

  const transitionName = await modVApp.evaluateWorker(
    () => self.store.getters["scenes/currentTransitionName"],
  );

  expect(transitionName).toBe("Fade");

  await modVApp.evaluateWorker(async () => {
    await self.store.dispatch("scenes/stopPerformance");
  });

  expect(await modVApp.evaluateWorkerState("scenes.autopilot.enabled")).toBe(
    false,
  );

  await removeScene(sceneA.id);
  await removeScene(sceneB.id);
  await ensureUserGroup();
});
