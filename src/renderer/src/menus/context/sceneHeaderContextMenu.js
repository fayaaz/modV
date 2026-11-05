import modV from "../../application/index";

const { ipcRenderer } = window.electron;

const isSceneApplied = (scene) => {
  const groups =
    (modV.store.state.groups && modV.store.state.groups.groups) || [];

  return groups.some(
    (group) =>
      group && group.sceneId === scene.id && group.transitionGhost !== true,
  );
};

const captureScene = async (scene) => {
  const result = await modV.store.dispatch("scenes/captureLiveStateToScene", {
    sceneId: scene.id,
  });

  if (!result || !result.scene) {
    return;
  }

  ipcRenderer.send("save-file", {
    what: "scene",
    name: result.scene.name,
    fileType: "json",
    project: modV.store.state.projects.currentProject,
    payload: JSON.stringify(result.scene),
  });
};

// Evaluated when the menu opens: greyed out while performance mode runs (the
// live layer is being transitioned, so capturing would save other rows'
// groups) and when the Scene is not applied.
export function SceneHeaderContextMenu({ scene }) {
  const { performance } = modV.store.state.scenes;
  const captureDisabled = !!(performance && performance.enabled);

  return [
    {
      label: scene.name,
      enabled: false,
    },
    {
      type: "separator",
    },
    {
      label: "Save current state into scene",
      enabled: !captureDisabled && isSceneApplied(scene),
      click() {
        return captureScene(scene);
      },
    },
  ];
}
