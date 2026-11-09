import { expect, test } from "@playwright/test";
import constants from "../../../../src/renderer/src/application/constants";
import { modVApp } from "../../pageObjectModel";

/**
 * Hovering a gallery item must enable its preview module and render it into
 * the item canvas. GalleryItem#focus() used to read the Group list from the
 * renderer's mirror store, which replays worker mutations asynchronously: a
 * stale mirror threw (or skipped commits) and the item showed only the
 * inherited live view, never the module's own render.
 */
async function hoverGalleryItem(page, searchTerm, itemName) {
  const searchInput = page.locator("input.gallery-search");
  await searchInput.click();
  await searchInput.fill(searchTerm);

  // Let Vue apply the filter and the list reflow before hovering: the
  // filtered items move under the pointer and a hover issued mid-refresh can
  // be cancelled by the resulting mouseleave. Re-hover until the worker has
  // actually enabled the preview module.
  await page.evaluate(
    () =>
      new Promise((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(resolve)),
      ),
  );

  const item = page
    .locator(".smooth-dnd-container .gallery-item", { hasText: itemName })
    .first();

  // GalleryItem#mounted creates its preview module asynchronously; hovering
  // before that finishes enables nothing. Wait for the module to exist in the
  // gallery Group before touching the item.
  await expect
    .poll(() =>
      modVApp.evaluateWorker(
        async ({ name, galleryGroupName }) => {
          const group = self.store.state.groups.groups.find(
            (candidate) => candidate.name === galleryGroupName,
          );

          return Boolean(
            group &&
            group.modules
              .map((id) => self.store.state.modules.active[id])
              .some((module) => module && module.$moduleName === name),
          );
        },
        { name: itemName, galleryGroupName: constants.GALLERY_GROUP_NAME },
      ),
    )
    .toBe(true);

  for (let attempt = 0; attempt < 4; attempt++) {
    await item.hover();

    const enabled = await modVApp.evaluateWorker(
      async ({ name, galleryGroupName }) => {
        const group = self.store.state.groups.groups.find(
          (candidate) => candidate.name === galleryGroupName,
        );

        return Boolean(
          group &&
          group.modules
            .map((id) => self.store.state.modules.active[id])
            .some(
              (module) =>
                module && module.$moduleName === name && module.meta.enabled,
            ),
        );
      },
      { name: itemName, galleryGroupName: constants.GALLERY_GROUP_NAME },
    );

    if (enabled) {
      return;
    }

    await page.waitForTimeout(250);
  }
}

async function sampleItemCanvas(itemName) {
  // The hover enables the preview module asynchronously (and startup work can
  // delay it), so poll the authoritative worker store until the module is
  // enabled, then sample the item canvas. The polling loop runs inside a
  // single worker evaluation.
  return modVApp.evaluateWorker(
    async ({ name, galleryGroupName }) => {
      const deadline = Date.now() + 30000;

      let galleryGroup = null;
      let enabledModules = [];
      let result = { opaque: 0, nonBlack: 0, total: 0, enabledModules };

      while (Date.now() < deadline) {
        galleryGroup = self.store.state.groups.groups.find(
          (group) => group.name === galleryGroupName,
        );

        enabledModules = galleryGroup
          ? galleryGroup.modules
              .map((id) => self.store.state.modules.active[id])
              .filter((mod) => mod && mod.meta.enabled)
              .map((mod) => mod.$moduleName)
          : [];

        const aux = Object.values(self.store.state.outputs.auxillary).find(
          (item) => item.name === `${name}-gallery`,
        );

        if (enabledModules.includes(name) && aux && aux.context) {
          const context = aux.context;
          const { width, height } = context.canvas;
          const data = context.getImageData(0, 0, width, height).data;

          let opaque = 0;
          let nonBlack = 0;

          for (let i = 0; i < data.length; i += 4) {
            if (data[i + 3] > 0) {
              opaque += 1;
            }

            if (data[i] + data[i + 1] + data[i + 2] > 12) {
              nonBlack += 1;
            }
          }

          result = { opaque, nonBlack, total: width * height, enabledModules };

          // The preview renders a frame or two after the module is enabled;
          // sample until the canvas has actually been painted.
          if (opaque > 0 || nonBlack > 0) {
            break;
          }
        }

        await new Promise((resolve) => setTimeout(resolve, 100));
      }

      return result;
    },
    { name: itemName, galleryGroupName: constants.GALLERY_GROUP_NAME },
  );
}

test.afterEach(async () => {
  // Moving the pointer off the Gallery leaves no preview module enabled for
  // later specs that share this app instance.
  await modVApp.page.mouse.move(0, 0);
});

test("a hovered gallery item renders its module preview", async () => {
  const { page } = modVApp;

  await hoverGalleryItem(page, "counter", "Counter");

  const result = await sampleItemCanvas("Counter");

  expect(result.enabledModules).toContain("Counter");

  // Counter paints its own opaque background: with no live composition the
  // preview canvas can only have been painted by the module itself.
  expect(result.opaque).toBeGreaterThan(result.total * 0.9);
});

test("an ISF gallery preview renders over the live composition", async () => {
  const { page } = modVApp;

  // A live composition for the gallery item to inherit as its backdrop.
  await modVApp.evaluateWorker(async () => {
    const group = await self.store.dispatch("groups/createGroup", {
      name: "PreviewBackdrop",
      enabled: 1,
      inherit: false,
      clearing: true,
    });

    const module = await self.store.dispatch("modules/makeActiveModule", {
      moduleName: "Counter",
      moduleMeta: { enabled: true },
    });

    self.store.commit("groups/ADD_MODULE_TO_GROUP", {
      moduleId: module.$id,
      groupId: group.id,
    });

    self.__probeBackdropGroupId = group.id;
  });

  try {
    await hoverGalleryItem(page, "bloom", "Bloom");

    const result = await sampleItemCanvas("Bloom");

    expect(result.enabledModules).toContain("Bloom");

    // The inherited backdrop is fed through Bloom: the preview must show the
    // module rendering over the live composition, not an empty canvas.
    expect(result.nonBlack).toBeGreaterThan(result.total * 0.1);
  } finally {
    // The app (and its store) is shared by every spec in the worker: remove
    // the backdrop group so it cannot pollute later specs.
    await modVApp.evaluateWorker(async () => {
      if (self.__probeBackdropGroupId) {
        await self.store.dispatch("groups/removeGroup", {
          groupId: self.__probeBackdropGroupId,
        });
      }
    });
  }
});
