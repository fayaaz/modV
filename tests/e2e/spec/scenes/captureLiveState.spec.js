import { expect, test } from "@playwright/test";
import constants from "../../../../src/renderer/src/application/constants";
import { modVApp } from "../../pageObjectModel";

const uniqueName = (prefix) => `${prefix} ${Date.now()}-${Math.random()}`;

test.beforeEach(async () => {
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
    {
      galleryGroupName: constants.GALLERY_GROUP_NAME,
    },
  );
});

async function removeSceneAndGroups(sceneId) {
  await modVApp.evaluateWorker(
    async ({ galleryName, sceneId }) => {
      await self.store.dispatch("scenes/removeScene", { sceneId });

      const userGroups = self.store.state.groups.groups.filter(
        (group) => group.name !== galleryName,
      );

      for (let i = 0; i < userGroups.length; i++) {
        await self.store.dispatch("groups/removeGroup", {
          groupId: userGroups[i].id,
        });
      }
    },
    { galleryName: constants.GALLERY_GROUP_NAME, sceneId },
  );
}

test("capturing writes the live state back into the Scene document", async () => {
  const name = uniqueName("E2E Capture");

  const result = await modVApp.evaluateWorker(
    async ({ name }) => {
      const group = await self.store.dispatch("groups/createGroup", {
        name,
        enabled: true,
      });

      const scene = await self.store.dispatch("scenes/saveCurrentAsScene", {
        name,
      });

      // A Scene that was never applied has nothing to capture.
      const notApplied = await self.store.dispatch(
        "scenes/captureLiveStateToScene",
        { sceneId: scene.id },
      );

      await self.store.dispatch("groups/removeGroup", { groupId: group.id });
      await self.store.dispatch("scenes/applyScene", { sceneId: scene.id });

      const appliedGroup = self.store.state.groups.groups.find(
        (item) => item.sceneId === scene.id,
      );

      await self.store.dispatch("groups/updateGroupInput", {
        groupId: appliedGroup.id,
        key: "alpha",
        data: 0.35,
      });

      const captured = await self.store.dispatch(
        "scenes/captureLiveStateToScene",
        { sceneId: scene.id },
      );

      const stored = self.store.state.scenes.scenes.find(
        (item) => item.id === scene.id,
      );

      return {
        notApplied,
        capturedGroup: captured.scene
          ? captured.scene.groups.find((item) => item && item.name === name)
          : null,
        storedGroup: stored
          ? stored.groups.find((item) => item && item.name === name)
          : null,
        sceneId: scene.id,
      };
    },
    { name },
  );

  expect(result.notApplied).toEqual({ error: "not-applied" });
  expect(result.capturedGroup).toBeTruthy();
  expect(result.capturedGroup.alpha).toBe(0.35);
  expect(result.capturedGroup.context).toBeUndefined();
  expect(result.storedGroup.alpha).toBe(0.35);

  await removeSceneAndGroups(result.sceneId);
});

test("capturing does not write other Scenes' groups and refuses during performance mode", async () => {
  const nameA = uniqueName("E2E Capture Other A");
  const nameB = uniqueName("E2E Capture Other B");

  const result = await modVApp.evaluateWorker(
    async ({ nameA, nameB }) => {
      await self.store.dispatch("groups/createGroup", {
        name: nameA,
        enabled: true,
      });

      const sceneA = await self.store.dispatch("scenes/saveCurrentAsScene", {
        name: nameA,
      });

      await self.store.dispatch("groups/removeGroup", {
        groupId: self.store.state.groups.groups.find(
          (group) => group.name === nameA,
        ).id,
      });

      await self.store.dispatch("groups/createGroup", {
        name: nameB,
        enabled: true,
      });

      const sceneB = await self.store.dispatch("scenes/saveCurrentAsScene", {
        name: nameB,
      });

      await self.store.dispatch("scenes/applyScene", { sceneId: sceneB.id });

      const appliedGroups = self.store.state.groups.groups.filter(
        (group) => group.sceneId === sceneB.id,
      );

      // While Scene B is applied, capturing A would be a stale overwrite.
      const wrongScene = await self.store.dispatch(
        "scenes/captureLiveStateToScene",
        { sceneId: sceneA.id },
      );

      await self.store.dispatch("scenes/startPerformance");

      const duringPerformance = await self.store.dispatch(
        "scenes/captureLiveStateToScene",
        { sceneId: sceneB.id },
      );

      await self.store.dispatch("scenes/stopPerformance");

      // stopPerformance leaves whatever Scene was last applied in place.
      await self.store.dispatch("scenes/applyScene", { sceneId: sceneB.id });

      const afterPerformance = await self.store.dispatch(
        "scenes/captureLiveStateToScene",
        { sceneId: sceneB.id },
      );

      const capturedGroupNames = afterPerformance.scene
        ? afterPerformance.scene.groups
            .filter(Boolean)
            .map((group) => group.name)
        : null;

      return {
        appliedGroupCount: appliedGroups.length,
        wrongScene,
        duringPerformance,
        appliedGroupNames: appliedGroups.map((group) => group.name),
        capturedGroupNames,
        sceneAId: sceneA.id,
        sceneBId: sceneB.id,
      };
    },
    { nameA, nameB },
  );

  expect(result.appliedGroupCount).toBeGreaterThan(0);
  expect(result.wrongScene).toEqual({ error: "not-applied" });
  expect(result.duringPerformance).toEqual({ error: "performance" });
  expect(result.capturedGroupNames.sort()).toEqual(
    result.appliedGroupNames.slice().sort(),
  );

  await removeSceneAndGroups(result.sceneAId);
  await removeSceneAndGroups(result.sceneBId);
});
