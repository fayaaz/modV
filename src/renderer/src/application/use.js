import installPlugin from "./install-plugin";

export default function use(type, extension) {
  switch (type) {
    case "plugin": {
      return installPlugin(extension);
    }

    case "control": {
      this.store.dispatch("controls/addControl", extension);
      break;
    }

    // case 'renderer': {
    //   store.commit('renderers/addRenderer', extension)
    //   break
    // }

    default: {
      return installPlugin(type || extension);
    }
  }
}
