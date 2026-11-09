import { expect, test } from "@playwright/test";
import constants, {
  GROUP_ENABLED,
} from "../../../../src/renderer/src/application/constants";
import { modVApp } from "../../pageObjectModel";

const uniqueName = (prefix) => `${prefix} ${Date.now()}-${Math.random()}`;

async function createGroup(name, options = {}) {
  return modVApp.evaluateWorker(
    async ({ name, enabled, options }) => {
      const group = await self.store.dispatch("groups/createGroup", {
        name,
        enabled,
        ...options,
      });

      return group.id;
    },
    { name, enabled: GROUP_ENABLED, options },
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

async function useNoTransition() {
  await modVApp.evaluateWorker(async () => {
    await self.store.dispatch("scenes/updatePerformance", {
      enabled: false,
      allowedTransitions: ["None"],
      transitionSeconds: 0.1,
      order: "in-order",
    });
  });
}

async function setRowNames(rowNames) {
  await modVApp.evaluateWorker(
    async ({ rowNames: names }) => {
      await self.store.dispatch("scenes/setRowNames", { rowNames: names });
    },
    { rowNames },
  );
}

async function openCellEditDialog(cell) {
  const { page } = modVApp;

  await page.evaluate(() => {
    const { Menu } = window.remote;

    window.__e2eMenuOriginal = Menu.buildFromTemplate;
    window.__e2eMenuTemplate = null;
    Menu.buildFromTemplate = (template) => {
      window.__e2eMenuTemplate = template;
      return { popup() {} };
    };
  });

  try {
    await cell.click({ button: "right" });

    await expect
      .poll(async () => page.evaluate(() => !!window.__e2eMenuTemplate))
      .toBe(true);

    await page.evaluate(async () => {
      const item = window.__e2eMenuTemplate.find(
        (entry) => entry.label === "Edit",
      );

      await item.click();
    });
  } finally {
    await page.evaluate(() => {
      if (window.__e2eMenuOriginal) {
        window.remote.Menu.buildFromTemplate = window.__e2eMenuOriginal;
      }

      delete window.__e2eMenuOriginal;
      delete window.__e2eMenuTemplate;
    });
  }

  const dialog = page.locator("dialog[open]");
  await expect(dialog).toBeVisible();

  return dialog;
}

async function getSceneOrder(sceneIds) {
  return modVApp.evaluateWorker(
    async ({ ids }) => {
      const scenes = self.store.state.scenes.scenes;

      return Object.fromEntries(
        ids.map((id) => [id, scenes.findIndex((scene) => scene.id === id)]),
      );
    },
    { ids: sceneIds },
  );
}

async function applyScene(sceneId) {
  await modVApp.evaluateWorker(
    async ({ sceneId }) => {
      await self.store.dispatch("scenes/applyScene", { sceneId });
    },
    { sceneId },
  );
}

async function applySceneRow(sceneId, rowIndex) {
  await modVApp.evaluateWorker(
    async ({ sceneId, rowIndex }) => {
      await self.store.dispatch("scenes/applySceneRow", {
        sceneId,
        rowIndex,
      });
    },
    { sceneId, rowIndex },
  );
}

async function getRowTransitionState(rowIndex) {
  return modVApp.evaluateWorker(
    async ({ rowIndex }) => {
      const transition = self.store.getters["scenes/rowTransition"](rowIndex);

      return {
        hasTransition: !!transition,
        progress: self.store.getters["scenes/rowTransitionProgress"](rowIndex),
      };
    },
    { rowIndex },
  );
}

async function getGridState() {
  return modVApp.evaluateWorker(
    async ({ galleryGroupName }) => {
      const groups = self.store.state.groups.groups
        .filter(
          (group) =>
            group.name !== galleryGroupName && group.transitionGhost !== true,
        )
        .map((group) => ({
          id: group.id,
          name: group.name,
          sceneRow: group.sceneRow,
          modules: [...group.modules],
          moduleNames: group.modules.map(
            (moduleId) =>
              self.store.state.modules.active[moduleId] &&
              self.store.state.modules.active[moduleId].$moduleName,
          ),
        }))
        .sort((a, b) => {
          if (a.sceneRow === b.sceneRow) {
            return 0;
          }

          return a.sceneRow - b.sceneRow;
        });

      return {
        groups,
        activeRows: JSON.parse(
          JSON.stringify(self.store.getters["scenes/activeRows"]),
        ),
      };
    },
    { galleryGroupName: constants.GALLERY_GROUP_NAME },
  );
}

test("applying a row Scene only replaces that row", async () => {
  const prefix = uniqueName("E2E Row Apply");

  await removeUserGroups();
  await useNoTransition();

  const a0 = await createGroup(`${prefix} A0`);
  await addModuleToGroup(a0, "Ball");
  const a1 = await createGroup(`${prefix} A1`);
  await addModuleToGroup(a1, "Bands");
  const sceneA = await saveScene(`${prefix} A`);

  await removeUserGroups();

  const b0 = await createGroup(`${prefix} B0`);
  await addModuleToGroup(b0, "Bands");
  const b1 = await createGroup(`${prefix} B1`);
  await addModuleToGroup(b1, "Ball");
  const sceneB = await saveScene(`${prefix} B`);

  await applyScene(sceneA.id);

  const before = await getGridState();

  expect(before.groups).toHaveLength(2);
  expect(before.groups[0]).toEqual(
    expect.objectContaining({
      name: `${prefix} A0`,
      sceneRow: 0,
      moduleNames: ["Ball"],
    }),
  );
  expect(before.groups[1]).toEqual(
    expect.objectContaining({
      name: `${prefix} A1`,
      sceneRow: 1,
      moduleNames: ["Bands"],
    }),
  );
  expect(before.activeRows[0].sceneId).toBe(sceneA.id);
  expect(before.activeRows[1].sceneId).toBe(sceneA.id);

  const rowOneGroupId = before.groups[1].id;
  const rowOneModules = before.groups[1].modules;

  await applySceneRow(sceneB.id, 0);

  const after = await getGridState();

  expect(after.groups[0].id).toBe(before.groups[0].id);
  expect(after.groups[0].name).toBe(`${prefix} B0`);
  expect(after.groups[0].moduleNames).toEqual(["Bands"]);
  expect(after.groups[0].modules).not.toEqual(before.groups[0].modules);
  expect(after.groups[1].id).toBe(rowOneGroupId);
  expect(after.groups[1].name).toBe(`${prefix} A1`);
  expect(after.groups[1].modules).toEqual(rowOneModules);
  expect(after.activeRows[0].sceneId).toBe(sceneB.id);
  expect(after.activeRows[1].sceneId).toBe(sceneA.id);

  await removeScene(sceneA.id);
  await removeScene(sceneB.id);
  await removeUserGroups();
  await ensureUserGroup();
});

test("a column Scene apply clears a row the Scene does not define", async () => {
  const prefix = uniqueName("E2E Column Rows");

  await removeUserGroups();
  await useNoTransition();

  const a0 = await createGroup(`${prefix} A0`);
  await addModuleToGroup(a0, "Ball");
  const a1 = await createGroup(`${prefix} A1`);
  await addModuleToGroup(a1, "Bands");
  const sceneA = await saveScene(`${prefix} A`);

  await removeUserGroups();

  const c0 = await createGroup(`${prefix} C0`);
  await addModuleToGroup(c0, "Ball");
  const sceneC = await saveScene(`${prefix} C`);

  await applyScene(sceneA.id);

  const afterA = await getGridState();

  expect(afterA.groups).toHaveLength(2);
  expect(afterA.activeRows[0]).toBeTruthy();
  expect(afterA.activeRows[1]).toBeTruthy();

  await applyScene(sceneC.id);

  const afterC = await getGridState();

  expect(afterC.groups).toHaveLength(1);
  expect(afterC.groups[0].name).toBe(`${prefix} C0`);
  expect(afterC.activeRows[0].sceneId).toBe(sceneC.id);
  expect(afterC.activeRows[1]).toBeFalsy();

  await removeScene(sceneA.id);
  await removeScene(sceneC.id);
  await removeUserGroups();
  await ensureUserGroup();
});

test("a row Scene apply fades in and finishes", async () => {
  const prefix = uniqueName("E2E Row Fade");

  await removeUserGroups();
  await useNoTransition();

  const a0 = await createGroup(`${prefix} A0`);
  await addModuleToGroup(a0, "Ball");
  const a1 = await createGroup(`${prefix} A1`);
  await addModuleToGroup(a1, "Bands");
  const sceneA = await saveScene(`${prefix} A`);

  await removeUserGroups();

  const b0 = await createGroup(`${prefix} B0`);
  await addModuleToGroup(b0, "Bands");
  const b1 = await createGroup(`${prefix} B1`);
  await addModuleToGroup(b1, "Ball");
  const sceneB = await saveScene(`${prefix} B`);

  await applyScene(sceneA.id);

  await modVApp.evaluateWorker(async () => {
    await self.store.dispatch("scenes/updatePerformance", {
      enabled: false,
      transitionSeconds: 0.5,
    });
  });

  const immediately = await modVApp.evaluateWorker(
    async ({ sceneId, rowIndex }) => {
      await self.store.dispatch("scenes/applySceneRow", {
        sceneId,
        rowIndex,
      });

      const transition = self.store.getters["scenes/rowTransition"](rowIndex);

      return {
        hasTransition: !!transition,
        progress: self.store.getters["scenes/rowTransitionProgress"](rowIndex),
      };
    },
    { sceneId: sceneB.id, rowIndex: 0 },
  );

  expect(immediately.hasTransition).toBe(true);
  expect(immediately.progress).toBeLessThan(1);

  await modVApp.evaluateWorker(async () => {
    await new Promise((resolve) => setTimeout(resolve, 800));
  });

  const after = await getRowTransitionState(0);

  expect(after.hasTransition).toBe(false);
  expect(after.progress).toBe(1);

  await removeScene(sceneA.id);
  await removeScene(sceneB.id);
  await removeUserGroups();
  await ensureUserGroup();
});

test("a row fade keeps the outgoing visuals animating", async () => {
  const prefix = uniqueName("E2E Row Fade Motion");

  await removeUserGroups();
  await useNoTransition();

  const a0 = await createGroup(`${prefix} A0`);
  await addModuleToGroup(a0, "Counter");
  const sceneA = await saveScene(`${prefix} A`);

  await removeUserGroups();

  const b0 = await createGroup(`${prefix} B0`);
  await addModuleToGroup(b0, "Bar");
  const sceneB = await saveScene(`${prefix} B`);

  await applyScene(sceneA.id);

  await modVApp.evaluateWorker(async () => {
    await self.store.dispatch("scenes/updatePerformance", {
      enabled: false,
      transitionSeconds: 2,
    });
  });

  const during = await modVApp.evaluateWorker(
    async ({ sceneId }) => {
      const nextFrame = () =>
        new Promise((resolve) => requestAnimationFrame(resolve));

      await self.store.dispatch("scenes/applySceneRow", {
        sceneId,
        rowIndex: 0,
      });

      const findGhost = () =>
        self.store.state.groups.groups.find(
          (group) => group.transitionGhost === true && group.sceneRow === 0,
        );
      const sample = (group) => {
        if (!group) {
          return null;
        }

        const module = self.store.state.modules.active[group.modules[0]];

        return {
          alpha: group.alpha,
          counter: module && module.data && module.data.counter,
          moduleName: module && module.$moduleName,
        };
      };

      const before = sample(findGhost());

      for (let i = 0; i < 10; i++) {
        await nextFrame();
      }

      return { before, after: sample(findGhost()) };
    },
    { sceneId: sceneB.id },
  );

  expect(during.before).toBeTruthy();
  expect(during.after).toBeTruthy();
  expect(during.before.moduleName).toBe("Counter");
  expect(during.after.counter).toBeGreaterThan(during.before.counter);
  expect(during.after.alpha).toBeLessThan(during.before.alpha);

  await modVApp.evaluateWorker(async () => {
    await new Promise((resolve) => setTimeout(resolve, 500));

    // Finish the fade deterministically in case the render loop is throttled.
    for (let i = 0; i < 30; i++) {
      await self.store.dispatch("scenes/tick", { delta: 100 });
    }
  });

  const remaining = await modVApp.evaluateWorker(
    () =>
      self.store.state.groups.groups.filter(
        (group) => group.transitionGhost === true,
      ).length,
  );

  expect(remaining).toBe(0);

  await removeScene(sceneA.id);
  await removeScene(sceneB.id);
  await removeUserGroups();
  await ensureUserGroup();
});

test("a background row fade is visible through the rows that inherit it", async () => {
  const prefix = uniqueName("E2E Row Inherit Fade");
  const redName = uniqueName("E2E Solid Red");
  const blueName = uniqueName("E2E Solid Blue");
  const noopName = uniqueName("E2E Noop");

  await removeUserGroups();
  await useNoTransition();

  await modVApp.evaluateWorker(
    async ({ redName, blueName, noopName }) => {
      const solid = (name, color) => ({
        meta: {
          name,
          author: "e2e",
          version: "1.0.0",
          type: "2d",
        },
        props: {},
        data: {},
        draw({ context, canvas }) {
          context.fillStyle = color;
          context.fillRect(0, 0, canvas.width, canvas.height);
        },
      });

      await self.store.dispatch("modules/registerModule", {
        module: solid(redName, "#ff0000"),
      });
      await self.store.dispatch("modules/registerModule", {
        module: solid(blueName, "#0000ff"),
      });
      await self.store.dispatch("modules/registerModule", {
        module: {
          meta: {
            name: noopName,
            author: "e2e",
            version: "1.0.0",
            type: "2d",
          },
          props: {},
          data: {},
          draw() {},
        },
      });
    },
    { redName, blueName, noopName },
  );

  const createInheritingScene = async (backgroundModule, suffix) => {
    const background = await createGroup(`${prefix} Background ${suffix}`, {
      inherit: false,
      clearing: true,
    });
    await addModuleToGroup(background, backgroundModule);

    const foreground = await createGroup(`${prefix} Foreground ${suffix}`, {
      inherit: true,
      clearing: true,
    });
    await addModuleToGroup(foreground, noopName);

    const scene = await saveScene(`${prefix} ${suffix}`);
    await removeUserGroups();

    return scene;
  };

  const sceneA = await createInheritingScene(redName, "A");
  const sceneB = await createInheritingScene(blueName, "B");

  const sampleInheritedRow = () =>
    modVApp.evaluateWorker(
      async ({ galleryGroupName }) => {
        const foreground = self.store.state.groups.groups.find(
          (group) =>
            group.name !== galleryGroupName &&
            group.transitionGhost !== true &&
            Number(group.sceneRow) === 1,
        );

        if (!foreground || !foreground.context || !foreground.context.context) {
          return null;
        }

        const context = foreground.context.context;
        const { width, height } = context.canvas;
        const pixel = context.getImageData(
          Math.floor(width / 2),
          Math.floor(height / 2),
          1,
          1,
        ).data;

        return Array.from(pixel);
      },
      { galleryGroupName: constants.GALLERY_GROUP_NAME },
    );

  const waitForFrames = () =>
    modVApp.evaluateWorker(
      () =>
        new Promise((resolve) => {
          requestAnimationFrame(() => requestAnimationFrame(resolve));
        }),
    );

  await applyScene(sceneA.id);

  let before = null;

  for (let i = 0; i < 60; i++) {
    await waitForFrames();
    before = await sampleInheritedRow();

    if (before && before[0] > 200) {
      break;
    }
  }

  expect(before).toBeTruthy();
  expect(before[0]).toBeGreaterThan(200);
  expect(before[2]).toBeLessThan(60);

  await modVApp.evaluateWorker(async () => {
    await self.store.dispatch("scenes/updatePerformance", {
      transitionSeconds: 10,
    });
  });

  await applySceneRow(sceneB.id, 0);
  await modVApp.evaluateWorker(async () => {
    await self.store.dispatch("scenes/tick", { delta: 3000 });
  });
  await waitForFrames();

  // The row below inherits the fading background. Part way through the fade it
  // must still show mostly the outgoing red, not the incoming blue immediately.
  const during = await sampleInheritedRow();

  expect(during).toBeTruthy();
  expect(during[0]).toBeGreaterThan(during[2]);

  await modVApp.evaluateWorker(async () => {
    for (let i = 0; i < 220; i++) {
      await self.store.dispatch("scenes/tick", { delta: 100 });
    }
  });
  await waitForFrames();

  const after = await sampleInheritedRow();

  expect(after).toBeTruthy();
  expect(after[2]).toBeGreaterThan(200);
  expect(after[0]).toBeLessThan(60);

  await removeScene(sceneA.id);
  await removeScene(sceneB.id);
  await removeUserGroups();
  await ensureUserGroup();

  await modVApp.evaluateWorker(
    async ({ moduleNames }) => {
      for (const moduleName of moduleNames) {
        self.store.commit("modules/REMOVE_REGISTERED_MODULE", { moduleName });
      }
    },
    { moduleNames: [redName, blueName, noopName] },
  );
});

test("the Scenes grid marks the applied row cell as active", async () => {
  const prefix = uniqueName("E2E Grid DOM");

  await removeUserGroups();
  await useNoTransition();

  const groupId = await createGroup(`${prefix} A0`);
  await addModuleToGroup(groupId, "Ball");
  const scene = await saveScene(`${prefix} A`);

  await applySceneRow(scene.id, 0);

  const { page } = modVApp;
  await page.locator('.lm_tab[title="Scenes"]').click();
  await expect(page.locator(".scenes__grid")).toBeVisible();

  const cell = page.locator(
    `.scenes__gridCell[data-scene-id="${scene.id}"][data-row-index="0"]`,
  );

  await expect(cell).toHaveClass(/scenes__gridCell--active/);

  await removeScene(scene.id);
  await removeUserGroups();
  await ensureUserGroup();
});

test("naming a row matches groups by name and reorders the layer stack", async () => {
  const prefix = uniqueName("E2E Row Names");

  await removeUserGroups();
  await useNoTransition();

  const a0 = await createGroup(`${prefix} A0`);
  await addModuleToGroup(a0, "Ball");
  const webcam = await createGroup("webcam");
  await addModuleToGroup(webcam, "Bands");
  const sceneA = await saveScene(`${prefix} A`);

  await removeUserGroups();

  const b0 = await createGroup(`${prefix} B0`);
  await addModuleToGroup(b0, "Ball");
  const bWebcam = await createGroup("WEBCAM");
  await addModuleToGroup(bWebcam, "Bands");
  const sceneB = await saveScene(`${prefix} B`);

  await applyScene(sceneA.id);

  await modVApp.evaluateWorker(
    async ({ rowNames }) => {
      await self.store.dispatch("scenes/setRowNames", { rowNames });
    },
    { rowNames: ["webcam"] },
  );

  const named = await getGridState();

  // The webcam Group snaps to the named row above the other layers.
  expect(named.groups[0].name).toBe("webcam");
  expect(named.groups[0].sceneRow).toBe(0);
  expect(named.groups[1].name).toBe(`${prefix} A0`);
  expect(named.groups[1].sceneRow).toBe(1);

  // Row 0 is now the webcam row; applying it across both Scenes swaps in the
  // Scene's first webcam-named group, and scenes without one stay empty.
  await applySceneRow(sceneB.id, 0);

  const afterB = await getGridState();

  expect(afterB.groups[0].name).toBe("WEBCAM");
  expect(afterB.groups[1].name).toBe(`${prefix} A0`);
  expect(afterB.activeRows[0].sceneId).toBe(sceneB.id);

  await removeScene(sceneA.id);
  await removeScene(sceneB.id);
  await removeUserGroups();
  await ensureUserGroup();

  await modVApp.evaluateWorker(async () => {
    await self.store.dispatch("scenes/setRowNames", { rowNames: [] });
  });
});

test("applying a Scene honours named rows instead of group positions", async () => {
  const prefix = uniqueName("E2E Named Apply");

  await removeUserGroups();
  await useNoTransition();
  await setRowNames([]);

  const cam = await createGroup("WEBCAM");
  await addModuleToGroup(cam, "Ball");
  const fg = await createGroup("Foreground");
  await addModuleToGroup(fg, "Bands");
  const fx = await createGroup("Effects");
  await addModuleToGroup(fx, "Bands");
  const scene = await saveScene(`${prefix} Scene`);

  await removeUserGroups();
  await setRowNames([
    "webcam",
    "Background",
    "Foreground",
    "Effects",
    "effects2",
  ]);
  await applyScene(scene.id);

  const state = await getGridState();
  const rowsByName = Object.fromEntries(
    state.groups.map((group) => [group.name, group.sceneRow]),
  );

  // The Scene's three Groups are pinned to the rows that display them, not
  // to their document positions (which would put Effects on row 2).
  expect(rowsByName).toMatchObject({
    WEBCAM: 0,
    Foreground: 2,
    Effects: 3,
  });
  expect(Object.keys(state.activeRows).sort()).toEqual(["0", "2", "3"]);

  await removeScene(scene.id);
  await removeUserGroups();
  await setRowNames([]);
});

test("an Input Link with no audio data leaves the prop value alone", async () => {
  await removeUserGroups();

  const groupId = await createGroup(uniqueName("E2E Missing Audio"));
  const moduleId = await addModuleToGroup(groupId, "Ball");

  const inputId = await modVApp.evaluateWorker(
    async ({ moduleId: id }) => {
      const module = self.store.state.modules.active[id];
      const inputId = module.$props.speed.id;

      await self.store.dispatch("inputs/createInputLink", {
        inputId,
        location: "meyda/getFeature",
        type: "getter",
        args: ["bass"],
        min: 1,
        max: 5,
        source: "meyda",
      });

      return inputId;
    },
    { moduleId },
  );

  // Let the render loop evaluate the link a few times. Meyda has no frame to
  // hand over without an audio input, so the link must keep the saved value
  // instead of writing NaN into it.
  await modVApp.page.waitForTimeout(300);

  const speed = await modVApp.evaluateWorker(
    async ({ moduleId: id, inputId: linkInputId }) => {
      const module = self.store.state.modules.active[id];

      return {
        value: module.props.speed,
        hasLink: !!self.store.state.inputs.inputLinks[linkInputId],
      };
    },
    { moduleId, inputId },
  );

  expect(speed.hasLink).toBe(true);
  expect(Number.isFinite(speed.value)).toBe(true);
  expect(speed.value).toBeGreaterThan(0);

  await removeUserGroups();
});

test("Ctrl+drag copies a cell's group into an empty cell of another scene", async () => {
  // The drag goes through the UI, which shares the GPU with the rest of the
  // suite, so give it more than the default budget.
  test.setTimeout(60000);

  const prefix = uniqueName("E2E Cell Copy");

  await removeUserGroups();
  await useNoTransition();

  const a0 = await createGroup(`${prefix} A0`);
  await addModuleToGroup(a0, "Ball");
  const a1 = await createGroup(`${prefix} A1`);
  await addModuleToGroup(a1, "Bands");
  const a2 = await createGroup(`${prefix} A2`);
  await addModuleToGroup(a2, "Ball");
  const sceneA = await saveScene(`${prefix} A`);

  await removeUserGroups();

  const b0 = await createGroup(`${prefix} B0`);
  await addModuleToGroup(b0, "Bands");
  const sceneB = await saveScene(`${prefix} B`);

  const { page } = modVApp;
  await page.locator('.lm_tab[title="Scenes"]').click();
  await expect(page.locator(".scenes__grid")).toBeVisible();

  const fromCell = `.scenes__gridCell[data-scene-id="${sceneA.id}"][data-row-index="2"]`;
  const toCell = `.scenes__gridCell[data-scene-id="${sceneB.id}"][data-row-index="2"]`;

  // The source cell holds a Group, the target cell is empty.
  await expect(page.locator(fromCell)).not.toHaveClass(
    /scenes__gridCell--empty/,
  );
  await expect(page.locator(toCell)).toHaveClass(/scenes__gridCell--empty/);

  const drag = await page.evaluate(
    ({ fromCell, toCell }) => {
      const from = document.querySelector(fromCell);
      const to = document.querySelector(toCell);
      const dataTransfer = new DataTransfer();
      const dispatch = (element, type) =>
        element.dispatchEvent(
          new DragEvent(type, {
            bubbles: true,
            cancelable: true,
            ctrlKey: true,
            dataTransfer,
          }),
        );

      dispatch(from, "dragstart");

      const overEvent = new DragEvent("dragover", {
        bubbles: true,
        cancelable: true,
        ctrlKey: true,
        dataTransfer,
      });
      to.dispatchEvent(overEvent);

      dispatch(to, "drop");

      return { dropAllowed: overEvent.defaultPrevented };
    },
    { fromCell, toCell },
  );

  // The browser only fires `drop` when dragover calls preventDefault.
  expect(drag.dropAllowed).toBe(true);

  // The drop handler dispatches to the worker asynchronously.
  await modVApp.evaluateWorker(async () => {
    await new Promise((resolve) => setTimeout(resolve, 500));
  });

  const { from, to } = await modVApp.evaluateWorker(
    async ({ fromSceneId, toSceneId }) => {
      const scenes = self.store.state.scenes.scenes;

      return {
        from: JSON.parse(
          JSON.stringify(scenes.find((scene) => scene.id === fromSceneId)),
        ),
        to: JSON.parse(
          JSON.stringify(scenes.find((scene) => scene.id === toSceneId)),
        ),
      };
    },
    { fromSceneId: sceneA.id, toSceneId: sceneB.id },
  );

  const sourceGroup = from.groups[2];

  expect(sourceGroup).toBeTruthy();
  expect(sourceGroup.name).toBe(`${prefix} A2`);

  // The empty row 2 receives the source Group verbatim; the rows above it
  // keep their place and row 1 is left as a hole.
  expect(to.groups).toHaveLength(3);
  expect(to.groups[1]).toBeNull();
  expect(to.groups[2]).toEqual(sourceGroup);
  expect(to.groups[2].modules).toEqual(sourceGroup.modules);
  // The copied module rides along (snapshot shape with saved props).
  expect(to.modules[sourceGroup.modules[0]]).toEqual(
    from.modules[sourceGroup.modules[0]],
  );

  await applyScene(sceneB.id);

  const after = await getGridState();

  // The hole keeps the rows aligned: the copy stays on row 2.
  const copied = after.groups.find((group) => group.name === `${prefix} A2`);

  expect(copied).toBeTruthy();
  expect(copied.sceneRow).toBe(2);
  expect(copied.moduleNames).toEqual(["Ball"]);

  await removeScene(sceneA.id);
  await removeScene(sceneB.id);
  await removeUserGroups();
  await ensureUserGroup();
});

test("hovering a grid cell previews the saved group and leaves a still", async () => {
  const prefix = uniqueName("E2E Grid Preview");

  await removeUserGroups();
  await useNoTransition();

  const firstGroupId = await createGroup(`${prefix} A0`);
  await addModuleToGroup(firstGroupId, "Ball");
  await addModuleToGroup(firstGroupId, "Bands");
  const secondGroupId = await createGroup(`${prefix} A1`);
  await addModuleToGroup(secondGroupId, "Ball");
  const scene = await saveScene(`${prefix} A`);

  const { page } = modVApp;
  await page.locator('.lm_tab[title="Scenes"]').click();
  await expect(page.locator(".scenes__grid")).toBeVisible();

  const firstCell = page.locator(
    `.scenes__gridCell[data-scene-id="${scene.id}"][data-row-index="0"]`,
  );
  const secondCell = page.locator(
    `.scenes__gridCell[data-scene-id="${scene.id}"][data-row-index="1"]`,
  );
  const firstPreview = firstCell.locator("canvas.scenes__gridPreviewCanvas");
  const secondPreview = secondCell.locator("canvas.scenes__gridPreviewCanvas");

  await firstCell.hover();
  await expect(firstPreview).toHaveClass(/scenes__gridPreviewCanvas--visible/);
  await expect(firstPreview).toBeVisible();
  await expect
    .poll(() => modVApp.evaluateWorkerState("scenePreview"))
    .toMatchObject({
      active: true,
      sceneId: scene.id,
      rowIndex: 0,
      moduleCount: 2,
      rendered: true,
    });

  await secondCell.hover();
  await expect(firstPreview).toHaveClass(/scenes__gridPreviewCanvas--visible/);
  await expect(firstPreview).toBeVisible();
  await expect(secondPreview).toHaveClass(/scenes__gridPreviewCanvas--visible/);
  await expect(secondPreview).toBeVisible();
  await expect
    .poll(() => modVApp.evaluateWorkerState("scenePreview"))
    .toMatchObject({
      active: true,
      sceneId: scene.id,
      rowIndex: 1,
      moduleCount: 1,
      rendered: true,
    });

  await page.locator(".scenes__gridCorner").hover();
  await expect(firstPreview).toHaveClass(/scenes__gridPreviewCanvas--visible/);
  await expect(secondPreview).toHaveClass(/scenes__gridPreviewCanvas--visible/);
  await expect
    .poll(() => modVApp.evaluateWorkerState("scenePreview"))
    .toMatchObject({
      active: false,
      frozen: true,
      rowIndex: 1,
      rendered: true,
    });

  await firstCell.hover();
  await expect
    .poll(() => modVApp.evaluateWorkerState("scenePreview"))
    .toMatchObject({
      active: true,
      sceneId: scene.id,
      rowIndex: 0,
      moduleCount: 2,
      rendered: true,
    });
  await expect(firstPreview).toHaveClass(/scenes__gridPreviewCanvas--visible/);
  await expect(secondPreview).toHaveClass(/scenes__gridPreviewCanvas--visible/);

  await removeScene(scene.id);
  await removeUserGroups();
  await ensureUserGroup();
});

test("the Scenes grid adds a new empty Scene column", async () => {
  const before = await modVApp.evaluateWorker(() =>
    self.store.state.scenes.scenes.map((scene) => ({
      id: scene.id,
      name: scene.name,
    })),
  );

  const { page } = modVApp;
  await page.locator('.lm_tab[title="Scenes"]').click();
  await expect(page.locator(".scenes__grid")).toBeVisible();

  await page
    .locator(".scenes__gridCornerButtons button", { hasText: "+ Column" })
    .click();

  await expect
    .poll(async () =>
      modVApp.evaluateWorker(() => self.store.state.scenes.scenes.length),
    )
    .toBe(before.length + 1);

  const beforeIds = before.map((scene) => scene.id);
  const added = await modVApp.evaluateWorker(
    ({ beforeIds: ids }) => {
      const scene = self.store.state.scenes.scenes.find(
        (item) => !ids.includes(item.id),
      );

      return scene ? JSON.parse(JSON.stringify(scene)) : null;
    },
    { beforeIds },
  );

  expect(added).toBeTruthy();
  expect(added.name).toMatch(/^Scene \d+$/);
  expect(added.groups).toEqual([]);
  expect(added.modules).toEqual({});

  await expect(page.locator(".scenes__gridHeader")).toHaveCount(
    before.length + 1,
  );
  await expect(
    page.locator(".scenes__gridHeaderName", { hasText: added.name }),
  ).toHaveCount(1);

  await removeScene(added.id);
});

test("editing a grid cell saves added modules back into the Scene", async () => {
  const prefix = uniqueName("E2E Cell Edit");

  await removeUserGroups();
  await useNoTransition();

  const groupId = await createGroup(`${prefix} A0`);
  await addModuleToGroup(groupId, "Ball");
  const scene = await saveScene(`${prefix} A`);

  await applyScene(scene.id);

  const { page } = modVApp;
  await page.locator('.lm_tab[title="Scenes"]').click();
  await expect(page.locator(".scenes__grid")).toBeVisible();

  const cell = page.locator(
    `.scenes__gridCell[data-scene-id="${scene.id}"][data-row-index="0"]`,
  );

  await expect(cell).not.toHaveClass(/scenes__gridCell--empty/);

  try {
    await page.evaluate(() => {
      const { Menu } = window.remote;

      window.__e2eMenuOriginal = Menu.buildFromTemplate;
      window.__e2eMenuTemplate = null;
      Menu.buildFromTemplate = (template) => {
        window.__e2eMenuTemplate = template;
        return { popup() {} };
      };
    });

    await cell.click({ button: "right" });

    await expect
      .poll(async () => page.evaluate(() => !!window.__e2eMenuTemplate))
      .toBe(true);

    const labels = await page.evaluate(() =>
      window.__e2eMenuTemplate
        .map((item) => item.label)
        .filter((label) => typeof label === "string"),
    );

    expect(labels).toEqual(expect.arrayContaining(["Edit", "Clear"]));

    await page.evaluate(async () => {
      const item = window.__e2eMenuTemplate.find(
        (entry) => entry.label === "Edit",
      );

      await item.click();
    });

    const dialog = page.locator("dialog[open]");
    await expect(dialog).toBeVisible();
    await expect(dialog.locator(".scenes__groupEditor .group")).toBeVisible();

    const beforeModules = await dialog.locator(".active-module").count();
    expect(beforeModules).toBe(1);

    await dialog.locator(".scenes__addModuleRow select").selectOption("Bands");
    await dialog.getByRole("button", { name: "Add module" }).click();
    await expect(dialog.locator(".active-module")).toHaveCount(
      beforeModules + 1,
    );

    await dialog.locator("header button.close").click();
    await expect(page.locator("dialog[open]")).toHaveCount(0);

    await expect
      .poll(() =>
        modVApp.evaluateWorker(
          async ({ sceneId }) => {
            const saved = self.store.state.scenes.scenes.find(
              (item) => item.id === sceneId,
            );

            if (!saved || !saved.groups[0]) {
              return null;
            }

            return saved.groups[0].modules.map(
              (moduleId) =>
                saved.modules[moduleId] && saved.modules[moduleId].$moduleName,
            );
          },
          { sceneId: scene.id },
        ),
      )
      .toEqual(["Ball", "Bands"]);
  } finally {
    await page.evaluate(() => {
      if (window.__e2eMenuOriginal) {
        window.remote.Menu.buildFromTemplate = window.__e2eMenuOriginal;
      }

      delete window.__e2eMenuOriginal;
      delete window.__e2eMenuTemplate;
    });
  }

  await removeScene(scene.id);
  await removeUserGroups();
  await ensureUserGroup();
});

test("clearing a grid cell removes its modules after confirmation", async () => {
  const prefix = uniqueName("E2E Cell Clear");

  await removeUserGroups();
  await useNoTransition();

  const groupId = await createGroup(`${prefix} A0`);
  await addModuleToGroup(groupId, "Ball");
  const scene = await saveScene(`${prefix} A`);

  await applyScene(scene.id);

  const moduleId = await modVApp.evaluateWorker(
    ({ sceneId }) =>
      self.store.state.scenes.scenes.find((item) => item.id === sceneId)
        .groups[0].modules[0],
    { sceneId: scene.id },
  );

  const { page } = modVApp;
  await page.locator('.lm_tab[title="Scenes"]').click();
  await expect(page.locator(".scenes__grid")).toBeVisible();

  const cell = page.locator(
    `.scenes__gridCell[data-scene-id="${scene.id}"][data-row-index="0"]`,
  );

  try {
    await page.evaluate(() => {
      const { Menu } = window.remote;

      window.__e2eMenuOriginal = Menu.buildFromTemplate;
      window.__e2eMenuTemplate = null;
      window.__e2eConfirmOriginal = window.confirm;
      window.__e2eConfirmCalls = [];
      window.confirm = (message) => {
        window.__e2eConfirmCalls.push(message);
        return true;
      };
      Menu.buildFromTemplate = (template) => {
        window.__e2eMenuTemplate = template;
        return { popup() {} };
      };
    });

    await cell.click({ button: "right" });

    await expect
      .poll(async () => page.evaluate(() => !!window.__e2eMenuTemplate))
      .toBe(true);

    await page.evaluate(async () => {
      const item = window.__e2eMenuTemplate.find(
        (entry) => entry.label === "Clear",
      );

      await item.click();
    });

    await expect
      .poll(async () => page.evaluate(() => window.__e2eConfirmCalls.length))
      .toBe(1);

    const confirmMessage = await page.evaluate(
      () => window.__e2eConfirmCalls[0],
    );
    expect(confirmMessage).toContain(`${prefix} A0`);
    expect(confirmMessage).toContain(scene.name);

    await expect
      .poll(() =>
        modVApp.evaluateWorker(
          ({ sceneId, moduleId: clearedModuleId }) => {
            const saved = self.store.state.scenes.scenes.find(
              (item) => item.id === sceneId,
            );
            const live = self.store.state.groups.groups.find(
              (group) =>
                group.sceneId === sceneId && group.transitionGhost !== true,
            );

            return {
              savedModules:
                saved && saved.groups[0]
                  ? saved.groups[0].modules.length
                  : null,
              savedSnapshot:
                saved && saved.modules
                  ? !!saved.modules[clearedModuleId]
                  : true,
              liveModules: live ? live.modules.length : null,
              activeModule: !!self.store.state.modules.active[clearedModuleId],
            };
          },
          { sceneId: scene.id, moduleId },
        ),
      )
      .toEqual({
        savedModules: 0,
        savedSnapshot: false,
        liveModules: 0,
        activeModule: false,
      });
  } finally {
    await page.evaluate(() => {
      if (window.__e2eMenuOriginal) {
        window.remote.Menu.buildFromTemplate = window.__e2eMenuOriginal;
      }

      if (window.__e2eConfirmOriginal) {
        window.confirm = window.__e2eConfirmOriginal;
      }

      delete window.__e2eMenuOriginal;
      delete window.__e2eMenuTemplate;
      delete window.__e2eConfirmOriginal;
      delete window.__e2eConfirmCalls;
    });
  }

  await removeScene(scene.id);
  await removeUserGroups();
  await ensureUserGroup();
});

test("renaming a Scene column updates the grid and the Scene", async () => {
  const prefix = uniqueName("E2E Column Rename");

  await removeUserGroups();
  await useNoTransition();

  const groupId = await createGroup(`${prefix} G`);
  await addModuleToGroup(groupId, "Ball");
  const scene = await saveScene(`${prefix} Old`);

  const { page } = modVApp;
  await page.locator('.lm_tab[title="Scenes"]').click();
  await expect(page.locator(".scenes__grid")).toBeVisible();

  const header = page.locator(
    `.scenes__gridHeader[data-scene-id="${scene.id}"]`,
  );
  const newName = `${prefix} New`;

  await header.locator(".scenes__gridHeaderName").dblclick();

  const nameInput = header.locator("input.scenes__sceneNameInput");
  await expect(nameInput).toBeVisible();
  await nameInput.fill(newName);
  await nameInput.press("Enter");

  await expect(header.locator(".scenes__gridHeaderName")).toHaveText(newName);
  await expect
    .poll(() =>
      modVApp.evaluateWorker(
        ({ sceneId }) => {
          const saved = self.store.state.scenes.scenes.find(
            (item) => item.id === sceneId,
          );

          return saved ? saved.name : null;
        },
        { sceneId: scene.id },
      ),
    )
    .toBe(newName);

  await removeScene(scene.id);
  await removeUserGroups();
  await ensureUserGroup();
});

test("dragging a Scene column reorders the Scenes", async () => {
  await removeUserGroups();
  await useNoTransition();

  const sceneA = await modVApp.evaluateWorker(() =>
    self.store.dispatch("scenes/addEmptyScene"),
  );
  const sceneB = await modVApp.evaluateWorker(() =>
    self.store.dispatch("scenes/addEmptyScene"),
  );

  const { page } = modVApp;
  await page.locator('.lm_tab[title="Scenes"]').click();
  await expect(page.locator(".scenes__grid")).toBeVisible();

  const before = await getSceneOrder([sceneA.id, sceneB.id]);
  expect(before[sceneA.id]).toBeLessThan(before[sceneB.id]);

  const drag = await page.evaluate(
    ({ fromId, toId }) => {
      const from = document.querySelector(
        `.scenes__gridHeader[data-scene-id="${fromId}"]`,
      );
      const to = document.querySelector(
        `.scenes__gridHeader[data-scene-id="${toId}"]`,
      );
      const dataTransfer = new DataTransfer();
      const dispatch = (element, type) =>
        element.dispatchEvent(
          new DragEvent(type, {
            bubbles: true,
            cancelable: true,
            dataTransfer,
          }),
        );

      dispatch(from, "dragstart");

      const overEvent = new DragEvent("dragover", {
        bubbles: true,
        cancelable: true,
        dataTransfer,
      });
      to.dispatchEvent(overEvent);

      dispatch(to, "drop");
      dispatch(from, "dragend");

      return {
        dropAllowed: overEvent.defaultPrevented,
        payload: dataTransfer.getData("text/plain"),
      };
    },
    { fromId: sceneA.id, toId: sceneB.id },
  );

  expect(drag.dropAllowed).toBe(true);
  expect(drag.payload).toBe(`scene:${sceneA.id}`);

  await expect
    .poll(async () => {
      const after = await getSceneOrder([sceneA.id, sceneB.id]);

      return after[sceneB.id] < after[sceneA.id];
    })
    .toBe(true);

  await removeScene(sceneA.id);
  await removeScene(sceneB.id);
  await removeUserGroups();
  await ensureUserGroup();
});

test("editing an empty Scene cell creates a Group and saves its modules", async () => {
  await removeUserGroups();
  await useNoTransition();

  const scene = await modVApp.evaluateWorker(() =>
    self.store.dispatch("scenes/addEmptyScene"),
  );

  const { page } = modVApp;
  await page.locator('.lm_tab[title="Scenes"]').click();
  await expect(page.locator(".scenes__grid")).toBeVisible();

  const cell = page.locator(
    `.scenes__gridCell[data-scene-id="${scene.id}"][data-row-index="0"]`,
  );
  await expect(cell).toHaveClass(/scenes__gridCell--empty/);

  const dialog = await openCellEditDialog(cell);

  await expect(dialog.locator(".scenes__groupEditor .group")).toBeVisible();
  await expect(dialog.locator(".active-module")).toHaveCount(0);

  await dialog.locator(".scenes__addModuleRow select").selectOption("Bands");
  await dialog.getByRole("button", { name: "Add module" }).click();
  await expect(dialog.locator(".active-module")).toHaveCount(1);

  await dialog.locator("header button.close").click();
  await expect(page.locator("dialog[open]")).toHaveCount(0);

  await expect
    .poll(() =>
      modVApp.evaluateWorker(
        ({ sceneId }) => {
          const saved = self.store.state.scenes.scenes.find(
            (item) => item.id === sceneId,
          );
          const group = saved && saved.groups[0];

          if (!group) {
            return null;
          }

          return {
            groupCount: saved.groups.filter(Boolean).length,
            moduleNames: group.modules.map(
              (moduleId) =>
                saved.modules[moduleId] && saved.modules[moduleId].$moduleName,
            ),
          };
        },
        { sceneId: scene.id },
      ),
    )
    .toMatchObject({ groupCount: 1, moduleNames: ["Bands"] });

  await removeScene(scene.id);
  await removeUserGroups();
  await ensureUserGroup();
});

test("the cell editor can drag a module from its panel into the Group", async () => {
  const prefix = uniqueName("E2E Cell Module Drag");

  await removeUserGroups();
  await useNoTransition();

  const groupId = await createGroup(`${prefix} G`);
  await addModuleToGroup(groupId, "Ball");
  const scene = await saveScene(`${prefix} Scene`);

  await applyScene(scene.id);

  const { page } = modVApp;
  await page.locator('.lm_tab[title="Scenes"]').click();
  await expect(page.locator(".scenes__grid")).toBeVisible();

  const cell = page.locator(
    `.scenes__gridCell[data-scene-id="${scene.id}"][data-row-index="0"]`,
  );
  const dialog = await openCellEditDialog(cell);

  await expect(dialog.locator(".active-module")).toHaveCount(1);

  const source = dialog.locator(
    '.scenes__modulePanelItem[data-module-name="Bands"]',
  );
  const target = dialog.locator(".group__modules");

  await expect(source).toBeVisible();
  await source.scrollIntoViewIfNeeded();
  await target.scrollIntoViewIfNeeded();

  const sourceBox = await source.boundingBox();
  const targetBox = await target.boundingBox();
  expect(sourceBox).toBeTruthy();
  expect(targetBox).toBeTruthy();

  await page.mouse.move(
    sourceBox.x + sourceBox.width / 2,
    sourceBox.y + sourceBox.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    targetBox.x + targetBox.width / 2,
    targetBox.y + targetBox.height / 2,
    { steps: 12 },
  );
  await page.mouse.move(
    targetBox.x + targetBox.width / 2,
    targetBox.y + targetBox.height / 2,
  );
  await page.mouse.up();

  await expect(dialog.locator(".active-module")).toHaveCount(2);

  await dialog.locator("header button.close").click();
  await expect(page.locator("dialog[open]")).toHaveCount(0);

  await expect
    .poll(() =>
      modVApp.evaluateWorker(
        ({ sceneId }) => {
          const saved = self.store.state.scenes.scenes.find(
            (item) => item.id === sceneId,
          );
          const group = saved && saved.groups[0];

          if (!group) {
            return null;
          }

          return group.modules.map(
            (moduleId) =>
              saved.modules[moduleId] && saved.modules[moduleId].$moduleName,
          );
        },
        { sceneId: scene.id },
      ),
    )
    .toEqual(["Ball", "Bands"]);

  await removeScene(scene.id);
  await removeUserGroups();
  await ensureUserGroup();
});
