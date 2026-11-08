import { expect, test } from "@playwright/test";
import constants, {
  GROUP_ENABLED,
} from "../../../../src/renderer/src/application/constants";
import { modVApp } from "../../pageObjectModel";

test("the Gallery group inherits only while an item is hovered", async () => {
  const gallery = await modVApp.evaluateWorker(
    async ({ galleryGroupName }) => {
      const group = self.store.state.groups.groups.find(
        (item) => item && item.name === galleryGroupName,
      );

      return group
        ? {
            inherit: group.inherit,
            enabled: group.enabled,
          }
        : null;
    },
    { galleryGroupName: constants.GALLERY_GROUP_NAME },
  );

  // The hovered item's canvas uses the inherited live composition as the
  // backdrop its preview modules render over.
  expect(gallery).toBeTruthy();
  expect(gallery.inherit).toBe(true);
});

test("the idle Gallery group canvas is not repainted every frame", async () => {
  const result = await modVApp.evaluateWorker(
    async ({ galleryGroupName, enabled }) => {
      // A live composition so there is real content to inherit.
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

      for (let i = 0; i < 15; i++) {
        await nextFrame();
      }

      const galleryGroup = self.store.state.groups.groups.find(
        (group) => group.name === galleryGroupName,
      );

      const { width, height } = galleryGroup.context.context.canvas;
      const sample = () =>
        galleryGroup.context.context.getImageData(
          Math.floor(width / 2),
          Math.floor(height / 2),
          1,
          1,
        ).data;

      const idle = Array.from(sample());

      // The inherited content exists: the live Group canvas is not empty.
      const liveSample = Array.from(
        liveGroup.context.context.getImageData(
          Math.floor(width / 2),
          Math.floor(height / 2),
          1,
          1,
        ).data,
      );

      await self.store.dispatch("groups/removeGroup", {
        groupId: liveGroup.id,
      });

      return { idle, liveSample };
    },
    {
      galleryGroupName: constants.GALLERY_GROUP_NAME,
      enabled: GROUP_ENABLED,
    },
  );

  // The live composition is rendering actual content...
  expect(result.liveSample[3]).toBeGreaterThan(0);

  // ...but with no hovered item the Gallery group canvas is left alone
  // (cleared, not repainted with the inherited frame every frame).
  expect(result.idle[3]).toBe(0);
});
