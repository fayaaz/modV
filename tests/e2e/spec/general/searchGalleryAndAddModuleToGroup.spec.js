import { expect, test } from "@playwright/test";
import constants from "../../../../src/renderer/src/application/constants";
import { modVApp } from "../../pageObjectModel";

test("search gallery and add module to group", async () => {
  const { page } = modVApp;

  // Later specs can leave the last-focused Group pointing at a Group they
  // removed; re-focus a live one so double-click has somewhere to add.
  const firstUserGroup = await modVApp.evaluateWorker(
    async ({ galleryGroupName }) =>
      self.store.state.groups.groups.find(
        (group) => group.name !== galleryGroupName && !group.hidden,
      ) ||
      (await self.store.dispatch("groups/createGroup", {
        name: "E2E Search Group",
        enabled: 1,
      })),
    { galleryGroupName: constants.GALLERY_GROUP_NAME },
  );

  await page.evaluate((groupId) => {
    const store = window.Vue.config.globalProperties.$store;
    store.dispatch("focus/setFocus", { id: groupId, type: "group" });
    store.commit("uiGroups/SET_LAST_FOCUSED", groupId);
  }, firstUserGroup.id);

  const searchInput = await page.locator("input.gallery-search");
  await searchInput.click();

  await searchInput.fill("ball");

  // Match the exact "Ball" module title: other Point10 scenes also contain the
  // word "Ball" (e.g. Ball Of Fire), so a substring match would pick the wrong
  // gallery item.
  await page
    .locator(".gallery-item")
    .filter({ has: page.locator(".title", { hasText: /^Ball$/ }) })
    .first()
    .dblclick({
      position: {
        x: 98,
        y: 27,
      },
    });

  await page.waitForSelector(".group__modules .active-module");
  const activeModules = await page.locator(".group__modules .active-module");
  expect(await activeModules.count()).toBe(1);

  const activeModule = await activeModules
    .first()
    .locator(".active-module__name");
  const activeModuleTitle = await activeModule.textContent();
  expect(activeModuleTitle.trim()).toBe("Ball");

  // Leave the shared Gallery unfiltered for later specs (drag-and-drop maps
  // its payload by the unfiltered module index).
  await searchInput.fill("");
});
