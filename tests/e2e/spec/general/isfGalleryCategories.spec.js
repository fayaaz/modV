import { expect, test } from "@playwright/test";
import { modVApp } from "../../pageObjectModel";

const sceneModules = [
  "digital-crystal-tunnel",
  "pixel-tunnel",
  "fractal-mage-tunnel",
  "lopy-frac-tunnel",
  "fractal-mountain-tunnel",
];

test("tunnel ISF modules are registered as 3D Scenes", async () => {
  const registered = await modVApp.evaluateWorkerState("modules.registered");

  for (const name of sceneModules) {
    expect(registered[name], name).toBeTruthy();
    expect(registered[name].meta.category, name).toBe("3d-scenes");
  }
});

test("tunnel ISF modules compile and draw", async () => {
  const { outcomes, errors } = await modVApp.evaluateWorker(
    async ({ names }) => {
      const outcomes = {};
      const errors = [];
      const originalError = console.error;

      console.error = (...args) => {
        errors.push(args.map((arg) => String(arg)).join(" "));
      };

      try {
        for (const name of names) {
          const module = await self.store.dispatch("modules/makeActiveModule", {
            moduleName: name,
          });
          const definition = self.store.state.modules.registered[name];

          const canvas = new OffscreenCanvas(64, 64);
          const context = canvas.getContext("2d");

          try {
            definition.draw({
              moduleId: module.$id,
              canvas,
              context,
              props: module.props,
              isGallery: false,
            });
            outcomes[name] = "ok";
          } catch (e) {
            outcomes[name] = String(e);
          }

          await self.store.dispatch("modules/removeActiveModule", {
            moduleId: module.$id,
          });
        }
      } finally {
        console.error = originalError;
      }

      return { outcomes, errors };
    },
    { names: sceneModules },
  );

  for (const name of sceneModules) {
    expect(outcomes[name], name).toBe("ok");
    expect(
      errors.filter((error) => error.includes(name)),
      name,
    ).toEqual([]);
  }
});

test("tunnel ISF modules render and animate", async () => {
  const size = 64;
  const results = await modVApp.evaluateWorker(
    async ({ names, size }) => {
      const outcomes = {};

      for (const name of names) {
        const module = await self.store.dispatch("modules/makeActiveModule", {
          moduleName: name,
        });
        const definition = self.store.state.modules.registered[name];
        const canvas = new OffscreenCanvas(size, size);
        const context = canvas.getContext("2d", { willReadFrequently: true });

        const draw = () => {
          context.clearRect(0, 0, size, size);
          definition.draw({
            moduleId: module.$id,
            canvas,
            context,
            props: module.props,
            isGallery: false,
          });
        };

        draw();
        const first = context.getImageData(0, 0, size, size).data.slice();

        // Advance TIME/TIMEDELTA and let persistent buffers settle.
        for (let i = 0; i < 10; i++) {
          await new Promise((resolve) => setTimeout(resolve, 40));
          draw();
        }

        const second = context.getImageData(0, 0, size, size).data.slice();

        let nonBlack = 0;
        let changed = 0;

        for (let i = 0; i < second.length; i += 4) {
          const luminance = second[i] + second[i + 1] + second[i + 2];

          if (luminance > 24) {
            nonBlack++;
          }

          const delta =
            Math.abs(first[i] - second[i]) +
            Math.abs(first[i + 1] - second[i + 1]) +
            Math.abs(first[i + 2] - second[i + 2]);

          if (delta > 12) {
            changed++;
          }
        }

        outcomes[name] = { nonBlack, changed, total: size * size };

        await self.store.dispatch("modules/removeActiveModule", {
          moduleId: module.$id,
        });
      }

      return outcomes;
    },
    { names: sceneModules, size },
  );

  for (const name of sceneModules) {
    const { nonBlack, changed, total } = results[name];

    expect(nonBlack, `${name} renders black`).toBeGreaterThan(total * 0.01);
    expect(changed, `${name} does not animate`).toBeGreaterThan(10);
  }
});
