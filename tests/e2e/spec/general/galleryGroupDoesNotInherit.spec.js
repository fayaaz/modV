import { expect, test } from "@playwright/test";
import constants from "../../../../src/renderer/src/application/constants";
import { modVApp } from "../../pageObjectModel";

test("the internal Gallery group does not inherit the previous row", async () => {
  const gallery = await modVApp.evaluateWorker(
    async ({ galleryGroupName }) => {
      const group = self.store.state.groups.groups.find(
        (item) => item && item.name === galleryGroupName,
      );

      return group
        ? {
            name: group.name,
            inherit: group.inherit,
            enabled: group.enabled,
          }
        : null;
    },
    { galleryGroupName: constants.GALLERY_GROUP_NAME },
  );

  expect(gallery).toBeTruthy();
  expect(gallery.inherit).toBe(false);
});
