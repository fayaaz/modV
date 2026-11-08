import modV from "../../application/index";
import { canCaptureScene } from "../../application/worker/scene-capture";

const { ipcRenderer } = window.electron;

const getLiveGroups = () =>
  (modV.store.state.groups && modV.store.state.groups.groups) || [];

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
// groups) and when the Scene cannot be captured into. An applied Scene takes
// its stamped Groups; an empty Scene column takes the current live layers.
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
      enabled: !captureDisabled && canCaptureScene(scene, getLiveGroups()),
      click() {
        return captureScene(scene);
      },
    },
  ];
}
