export default {
  folder: "scene",
  identifier: "🎬",

  fileTypes: ["json"],

  process() {
    return new Promise((resolve) => {
      resolve(true);
    });
  },
};
