import { expect, test } from "@playwright/test";
import constants, {
  GROUP_ENABLED,
} from "../../../../src/renderer/src/application/constants";
import { modVApp } from "../../pageObjectModel";

/**
 * A hovered Gallery item previews the live composition: the item canvas
 * receives the inherited frame and the preview module renders over it.
 * Modules that sample their input (Bloom and most ISF filters) would render
 * black without that inherited backdrop.
 */
test("a gallery item preview renders over the inherited composition", async () => {
  const result = await modVApp.evaluateWorker(
    async ({ galleryGroupName, enabled }) => {
      const size = 200;

      const liveGroup = await self.store.dispatch("groups/createGroup", {
        name: "ProbeLiveBackground",
        enabled,
        inherit: false,
        clearing: true,
      });

      const liveModule = await self.store.dispatch("modules/makeActiveModule", {
        moduleName: "Counter",
        moduleMeta: { enabled: true },
      });

      self.store.commit("groups/ADD_MODULE_TO_GROUP", {
        moduleId: liveModule.$id,
        groupId: liveGroup.id,
      });

      const nextFrame = () =>
        new Promise((resolve) => requestAnimationFrame(resolve));

      for (let i = 0; i < 10; i++) {
        await nextFrame();
      }

      const canvas = new OffscreenCanvas(size, size);
      const context = canvas.getContext("2d");
      context.fillStyle = "#000";
      context.fillRect(0, 0, size, size);

      const aux = await self.store.dispatch("outputs/getAuxillaryOutput", {
        name: "probe-Bloom-gallery",
        canvas,
        context,
        reactToResize: false,
        width: size,
        height: size,
      });

      const module = await self.store.dispatch("modules/makeActiveModule", {
        moduleName: "Bloom",
        moduleMeta: { enabled: false, isGallery: true },
        skipInit: true,
      });

      await self.store.dispatch("modules/init", {
        moduleId: module.$id,
        width: size,
        height: size,
      });

      await self.store.dispatch("modules/resize", {
        moduleId: module.$id,
        width: size,
        height: size,
      });

      const galleryGroup = self.store.state.groups.groups.find(
        (group) => group.name === galleryGroupName,
      );

      self.store.commit("groups/ADD_MODULE_TO_GROUP", {
        moduleId: module.$id,
        groupId: galleryGroup.id,
      });

      self.store.commit("groups/UPDATE_GROUP", {
        groupId: galleryGroup.id,
        data: { drawToCanvasId: aux.id, enabled },
      });

      self.store.commit("modules/UPDATE_ACTIVE_MODULE_META", {
        id: module.$id,
        metaKey: "enabled",
        data: true,
      });

      for (let i = 0; i < 30; i++) {
        await nextFrame();
      }

      const data = context.getImageData(0, 0, size, size).data;
      let nonBlack = 0;

      for (let i = 0; i < data.length; i += 4) {
        if (data[i] + data[i + 1] + data[i + 2] > 12) {
          nonBlack += 1;
        }
      }

      await self.store.dispatch("groups/removeGroup", {
        groupId: liveGroup.id,
      });

      return { nonBlack, total: size * size };
    },
    {
      galleryGroupName: constants.GALLERY_GROUP_NAME,
      enabled: GROUP_ENABLED,
    },
  );

  expect(result.nonBlack).toBeGreaterThan(result.total * 0.5);
});
