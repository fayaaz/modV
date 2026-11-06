import { expect, test } from "@playwright/test";
import constants from "../../../../src/renderer/src/application/constants";
import { modVApp } from "../../pageObjectModel";

const uniqueName = (prefix) => `${prefix} ${Date.now()}-${Math.random()}`;

/**
 * A saved layout can omit the Gallery panel, so the internal Gallery Group is
 * never created. Named-row Scene applies call groups/orderByIds; it used to
 * append `undefined` when that Group was missing, which corrupted the live
 * Group list and made performance rows (including Effects) stop applying.
 */
test("performance rows keep applying without a Gallery Group", async () => {
  test.setTimeout(60000);

  const prefix = uniqueName("E2E No Gallery");
  const sceneName = `${prefix} Scene`;

  const result = await modVApp.evaluateWorker(
    async ({ prefix: namePrefix, sceneName: sceneToSave, galleryName }) => {
      const waitFrames = async (count) => {
        const nextFrame = () =>
          new Promise((resolve) => requestAnimationFrame(resolve));

        for (let i = 0; i < count; i++) {
          await nextFrame();
        }
      };

      const removeUserGroups = async () => {
        const groups = self.store.state.groups.groups.filter(
          (group) => group && group.name !== galleryName,
        );

        for (let i = 0; i < groups.length; i++) {
          await self.store.dispatch("groups/removeGroup", {
            groupId: groups[i].id,
          });
        }
      };

      const createGroup = async (name) => {
        const group = await self.store.dispatch("groups/createGroup", {
          name,
          enabled: 1,
        });

        return group.id;
      };

      const addModule = async (groupId, moduleName) => {
        const module = await self.store.dispatch("modules/makeActiveModule", {
          moduleName,
          moduleMeta: { enabled: true },
        });

        self.store.commit("groups/ADD_MODULE_TO_GROUP", {
          moduleId: module.$id,
          groupId,
        });

        return module.$id;
      };

      const sampleMain = () => {
        const main = self.store.state.outputs.main;
        const { width, height } = main.canvas;
        const data = main.getImageData(0, 0, width, height).data;
        let hash = 2166136261;

        for (let i = 0; i < data.length; i += 16) {
          hash ^= data[i] | (data[i + 1] << 8) | (data[i + 2] << 16);
          hash = Math.imul(hash, 16777619) >>> 0;
        }

        return hash;
      };

      const setEffectsAlpha = (value) => {
        const group = self.store.state.groups.groups.find(
          (item) => item && item.name === "Effects",
        );

        self.store.commit("groups/UPDATE_GROUP", {
          groupId: group.id,
          data: { alpha: value },
        });
      };

      let sceneId = null;
      let galleryGroup = null;

      try {
        await self.store.dispatch("scenes/stopPerformance");
        await removeUserGroups();

        const background = await createGroup("Background");
        await addModule(background, "Plasma");
        const foreground = await createGroup("Foreground");
        await addModule(foreground, "Ball");
        const effects = await createGroup("Effects");
        await addModule(effects, "Color Invert");

        const scene = await self.store.dispatch("scenes/saveCurrentAsScene", {
          name: sceneToSave,
        });

        sceneId = scene.id;

        await removeUserGroups();

        // Simulate a layout that never mounted the Gallery panel. Remove it
        // with a plain commit so the Gallery component keeps a valid group id
        // and the same Group object can be restored afterwards.
        galleryGroup = self.store.state.groups.groups.find(
          (group) => group && group.name === galleryName,
        );

        if (galleryGroup) {
          self.store.commit("groups/REMOVE_GROUP", { id: galleryGroup.id });
        }

        await self.store.dispatch("scenes/setRowNames", {
          rowNames: ["webcam", "Background", "Foreground", "Effects"],
        });
        await self.store.dispatch("scenes/updatePerformance", {
          enabled: false,
          allowedTransitions: ["None"],
          transitionSeconds: 0.1,
        });

        await self.store.dispatch("scenes/applyScene", { sceneId });
        await waitFrames(15);

        const effectsGroup = self.store.state.groups.groups.find(
          (group) => group && group.name === "Effects",
        );

        if (!effectsGroup) {
          throw new Error("The Effects row was not applied");
        }

        const withEffects = sampleMain();
        setEffectsAlpha(0);
        await waitFrames(5);
        const withoutEffects = sampleMain();
        setEffectsAlpha(1);
        await waitFrames(5);

        await self.store.dispatch("scenes/setPerformanceEnabled", {
          enabled: true,
        });
        await self.store.dispatch("scenes/tick", { delta: 16 });

        return {
          hasUndefinedGroups: self.store.state.groups.groups.some(
            (group) => !group,
          ),
          effectsSceneRow: effectsGroup.sceneRow,
          effectsModules: effectsGroup.modules.map((id) => {
            const module = self.store.state.modules.active[id];

            return module && module.$moduleName;
          }),
          withEffects,
          withoutEffects,
          rowCount: self.store.getters["scenes/rowCount"],
        };
      } finally {
        await self.store.dispatch("scenes/stopPerformance");

        if (sceneId) {
          await self.store.dispatch("scenes/removeScene", { sceneId });
        }

        await removeUserGroups();

        const hasGallery = self.store.state.groups.groups.some(
          (group) => group && group.name === galleryName,
        );

        if (!hasGallery && galleryGroup) {
          self.store.commit("groups/ADD_GROUP", { group: galleryGroup });
        }

        await self.store.dispatch("scenes/setRowNames", { rowNames: [] });

        const hasUserGroup = self.store.state.groups.groups.some(
          (group) => group && group.name !== galleryName,
        );

        if (!hasUserGroup) {
          await self.store.dispatch("groups/createGroup", {
            name: `${namePrefix} Restore`,
            enabled: 1,
          });
        }
      }
    },
    {
      prefix,
      sceneName,
      galleryName: constants.GALLERY_GROUP_NAME,
    },
  );

  expect(result.hasUndefinedGroups).toBe(false);
  expect(result.effectsSceneRow).toBe(3);
  expect(result.effectsModules).toContain("Color Invert");
  expect(result.rowCount).toBeGreaterThanOrEqual(4);
  expect(result.withEffects).not.toBe(result.withoutEffects);
});
