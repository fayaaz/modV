import { expect, test } from "@playwright/test";
import { modVApp } from "../../pageObjectModel";

test("search gallery and add module to group", async () => {
  const { page } = modVApp;

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
});
