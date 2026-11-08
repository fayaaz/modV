import { expect, test } from "@playwright/test";
import constants, {
  GROUP_ENABLED,
} from "../../../../src/renderer/src/application/constants";
import { modVApp } from "../../pageObjectModel";

/**
 * Every frame the render loop calls updateModule for modules that expose an
 * `update` lifecycle. Modules that mutate and return the same data object do
 * not need a store commit: the store already holds the same (mutated) object.
 * Committing anyway re-runs the mutation pipeline and Vue reactivity per
 * module per frame.
 */
test("module data commits are skipped when update returns the same object", async () => {
  const result = await modVApp.evaluateWorker(
    async ({ galleryGroupName, enabled, prefix }) => {
      const sameRefName = `${prefix} same ref`;
      const newRefName = `${prefix} new ref`;

      await self.store.dispatch("modules/registerModule", {
        module: {
          meta: {
            name: sameRefName,
            author: "e2e",
            version: "1.0.0",
            type: "2d",
          },
          props: {},
          data: { ticks: 0 },
          update({ data }) {
            data.ticks += 1;
            return { data };
          },
          draw() {},
        },
      });

      await self.store.dispatch("modules/registerModule", {
        module: {
          meta: {
            name: newRefName,
            author: "e2e",
            version: "1.0.0",
            type: "2d",
          },
          props: {},
          data: { ticks: 0 },
          update({ data }) {
            return { data: { ticks: data.ticks + 1 } };
          },
          draw() {},
        },
      });

      const userGroups = self.store.state.groups.groups.filter(
        (group) => group.name !== galleryGroupName,
      );

      for (let i = 0; i < userGroups.length; i++) {
        await self.store.dispatch("groups/removeGroup", {
          groupId: userGroups[i].id,
        });
      }

      const group = await self.store.dispatch("groups/createGroup", {
        name: `${prefix} group`,
        enabled,
      });

      const sameRef = await self.store.dispatch("modules/makeActiveModule", {
        moduleName: sameRefName,
        moduleMeta: { enabled: true },
      });
      const newRef = await self.store.dispatch("modules/makeActiveModule", {
        moduleName: newRefName,
        moduleMeta: { enabled: true },
      });

      self.store.commit("groups/ADD_MODULE_TO_GROUP", {
        moduleId: sameRef.$id,
        groupId: group.id,
      });
      self.store.commit("groups/ADD_MODULE_TO_GROUP", {
        moduleId: newRef.$id,
        groupId: group.id,
      });

      const counts = { [sameRef.$id]: 0, [newRef.$id]: 0 };
      const unsubscribe = self.store.subscribe((mutation) => {
        if (
          mutation.type === "modules/UPDATE_ACTIVE_MODULE" &&
          mutation.payload.key === "data" &&
          mutation.payload.id in counts
        ) {
          counts[mutation.payload.id] += 1;
        }
      });

      const nextFrame = () =>
        new Promise((resolve) => requestAnimationFrame(resolve));

      for (let i = 0; i < 30; i++) {
        await nextFrame();
      }

      unsubscribe();

      return {
        counts,
        ids: { sameRef: sameRef.$id, newRef: newRef.$id },
        ticks: {
          sameRef: self.store.state.modules.active[sameRef.$id].data.ticks,
          newRef: self.store.state.modules.active[newRef.$id].data.ticks,
        },
      };
    },
    {
      galleryGroupName: constants.GALLERY_GROUP_NAME,
      enabled: GROUP_ENABLED,
      prefix: `E2E Commit Skip ${Date.now()}-${Math.random()}`,
    },
  );

  // Both modules actually updated every frame...
  expect(result.ticks.sameRef).toBeGreaterThan(10);
  expect(result.ticks.newRef).toBeGreaterThan(10);

  // ...but only the module that returns a new object needs a commit.
  expect(result.counts[result.ids.sameRef]).toBe(0);
  expect(result.counts[result.ids.newRef]).toBeGreaterThan(10);

  await modVApp.evaluateWorker(
    async ({ galleryGroupName }) => {
      const userGroups = self.store.state.groups.groups.filter(
        (group) => group.name !== galleryGroupName,
      );

      for (let i = 0; i < userGroups.length; i++) {
        await self.store.dispatch("groups/removeGroup", {
          groupId: userGroups[i].id,
        });
      }
    },
    { galleryGroupName: constants.GALLERY_GROUP_NAME },
  );
});
