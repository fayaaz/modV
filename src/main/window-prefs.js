import { app, dialog, ipcMain, shell } from "electron";
import os from "node:os";
import fs from "node:fs";
import { join } from "path";

import store from "./media-manager/store";
// import { autoUpdater } from "electron-updater"; @TODO add back in updater
import { checkMediaPermission } from "./check-media-permission";
import { createLogger, getLogDirectory } from "./logger";
import { setProjectNames, setCurrentProject } from "./projects";
import { closeWindow, createWindow, windows } from "./windows";
import { updateMenu } from "./menu-bar";
import { getMediaManager } from "./media-manager";
import { isQuitting, markQuitting, runCleanup, withTimeout } from "./shutdown";
import {
  getNextOutputWindowPlacement,
  placeMainWindow,
  prepareMainWindowPlacement,
  trackMainWindow,
  trackOutputWindow,
} from "./window-state";

const isDevelopment = !app.isPackaged;
const isTest = process.env.CI === "e2e";
let modVReady = false;
let destroyedListener = null;
let mainWindowClosing = false;

function waitForRendererDestroyed(timeoutMs = 1500) {
  if (destroyedListener) {
    ipcMain.removeListener("destroyed", destroyedListener);
    destroyedListener = null;
  }

  let resolveDestroyed;
  const destroyed = new Promise((resolve) => {
    resolveDestroyed = resolve;
  });

  const listener = () => resolveDestroyed();

  destroyedListener = listener;
  ipcMain.on("destroyed", listener);

  return withTimeout(destroyed, timeoutMs).finally(() => {
    ipcMain.removeListener("destroyed", listener);

    if (destroyedListener === listener) {
      destroyedListener = null;
    }
  });
}

async function closeMainWindow(window) {
  // 0 because this is the position of the buttons in the dialog's buttons array
  let shouldQuit = 0;

  if (!isDevelopment && !isTest) {
    const { response } = await dialog.showMessageBox(window, {
      type: "question",
      buttons: ["Yes", "No"],
      message: "VmodV",
      detail: "Are you sure you want to quit?",
    });

    shouldQuit = response;
  }

  if (shouldQuit !== 0) {
    mainWindowClosing = false;
    return;
  }

  markQuitting();

  console.log("main sending webcontents modv-destroy");

  try {
    if (!window.isDestroyed()) {
      window.webContents.send("modv-destroy");
    }
  } catch (e) {
    // The renderer may already be gone.
  }

  await waitForRendererDestroyed(1500);
  await runCleanup();

  // Let before-quit/will-quit run so the shutdown coordinator and the audio
  // cleanup can finish. The coordinator's watchdog force-exits if they hang.
  app.quit();
}

const windowPrefs = {
  colorPicker: {
    devPath: "/color-picker.html",
    prodPath: "color-picker.html",
    options: {
      webPreferences: {
        preload: join(__dirname, "../preload/colorPicker.js"),
        contextIsolation: false,
        nodeIntegration: false,
      },
      transparent: true,
      frame: false,
      alwaysOnTop: true,
      resizable: false,
      skipTaskbar: true,
      fullscreenable: false,
    },
    unique: true,
    create(window) {
      window.on("close", (e) => {
        if (isQuitting()) {
          return;
        }

        e.preventDefault();

        window.hide();
      });

      window.on("blur", () => {
        window.hide();
      });
    },
  },

  mainWindow: {
    devPath: "/index.html",
    prodPath: "index.html",
    options: {
      show: isDevelopment,
      webPreferences: {
        preload: join(__dirname, "../preload/index.js"),
        enableRemoteModule: true,
        // electron 12 sets contextIsolation to true by default, this breaks VmodV
        contextIsolation: false,
        nodeIntegration: true,
        nodeIntegrationInWorker: true,
        nativeWindowOpen: true, // window.open return Window object(like in regular browsers), not BrowserWindowProxy
        affinity: "main-window", // main window, and additional windows should work in one process
      },
    },
    unique: true,

    beforeCreate() {
      const { bounds, restored } = prepareMainWindowPlacement();

      modVReady = false;

      const options = {
        width: bounds.width,
        height: bounds.height,
      };

      // Seed the constructor position too so the window does not appear in
      // the window manager's default spot before the exact bounds are applied.
      if (restored) {
        options.x = bounds.x;
        options.y = bounds.y;
      }

      return { options };
    },

    /**
     * @param {Electron.BrowserWindow} window
     */
    async create(window) {
      require("@electron/remote/main").enable(window.webContents);

      ipcMain.handle("is-modv-ready", () => modVReady);

      window.setRepresentedFilename(os.homedir());
      window.setDocumentEdited(true);
      window.setTitle("VmodV");

      window.webContents.on("did-finish-load", () => {
        window.setTitle("VmodV");
      });

      // Configure child windows to open without a menubar (windows/linux).
      // Electron 45 requires the child's sandbox/nodeIntegration state to
      // match the opener, otherwise the relationship is dropped and
      // window.open() returns null.
      window.webContents.setWindowOpenHandler(({ frameName }) => {
        if (frameName !== "modal") {
          return { action: "deny" };
        }

        // Output windows remember where they were (which screen and which
        // resolution) between restarts. Seeding the options avoids the window
        // painting at the default position before did-create-window can move
        // it.
        const placement = getNextOutputWindowPlacement();

        return {
          action: "allow",
          overrideBrowserWindowOptions: {
            autoHideMenuBar: true,
            enableLargerThanScreen: true,
            title: "",
            ...(placement.savedBounds || {}),
            webPreferences: {
              sandbox: false,
              contextIsolation: false,
              nodeIntegration: true,
              nodeIntegrationInWorker: true,
            },
          },
        };
      });

      // Attach the placement bookkeeping once the child window exists. The
      // saved bounds are applied again here because window.open()'s feature
      // string (width/height/left) can win over the override options above.
      window.webContents.on("did-create-window", (childWindow) => {
        trackOutputWindow(childWindow);
      });

      trackMainWindow(window);

      const mm = getMediaManager();

      mm.update = (message) => {
        window.webContents.send("media-manager-update", message);

        setProjectNames(mm.$store.getters["media/projects"]);
        updateMenu();
      };

      mm.pathChanged = (message) => {
        window.webContents.send("media-manager-path-changed", message);
      };

      ipcMain.on("open-window", (event, message) => {
        createWindow({ windowName: message }, event);
      });

      ipcMain.on("close-window", (event, message) => {
        closeWindow({ windowName: message });
      });

      ipcMain.on("modv-ready", () => {
        modVReady = true;
        mm.start();
      });

      ipcMain.on("modv-destroy", () => {
        mm.reset();
      });

      ipcMain.on("get-media-manager-state", (event) => {
        event.reply(
          "media-manager-state",
          JSON.parse(JSON.stringify(store.state.media)),
        );
      });

      ipcMain.on("save-file", async (event, message) => {
        try {
          await mm.saveFile(message);
        } catch (e) {
          event.reply("save-file", e);
          return;
        }

        event.reply("save-file", "saved");
      });

      ipcMain.on("delete-scene", async (event, { name, project }) => {
        try {
          await fs.promises.unlink(
            join(mm.mediaDirectoryPath, project, "scene", `${name}.json`),
          );
          event.reply("delete-scene", "deleted");
        } catch (e) {
          event.reply("delete-scene", e);
        }
      });

      ipcMain.on("current-project", (event, message) => {
        setCurrentProject(message);
        updateMenu();
      });

      ipcMain.on("input-update", (event, message) => {
        window.webContents.send("input-update", message);
      });

      // The renderer has no way to write to the log directory itself, so it
      // forwards its log lines here and they all end up in the same file. The
      // renderer already printed them to its own console.
      ipcMain.on("log", (event, payload) => {
        const { level, scope, message, data } = payload || {};
        const rendererLogger = createLogger(scope || "renderer", {
          mirror: false,
        });

        if (level === "error") {
          rendererLogger.error(message, data);
        } else if (level === "warn") {
          rendererLogger.warn(message, data);
        } else {
          rendererLogger.info(message, data);
        }
      });

      ipcMain.handle("get-logs-folder", async () => {
        return getLogDirectory();
      });

      ipcMain.handle("open-logs-folder", async () => {
        const directory = getLogDirectory();

        try {
          await shell.openPath(directory);
        } catch (e) {
          createLogger("main:logs").error("could not open the logs folder", e);
        }

        return directory;
      });

      ipcMain.handle("export-scenes-playlist", async (event, playlist) => {
        const result = await dialog.showSaveDialog(window, {
          title: "Export Scenes Playlist",
          defaultPath: "scenes-playlist.json",
          filters: [{ name: "Scenes Playlist", extensions: ["json"] }],
        });

        if (result.canceled || !result.filePath) {
          return { canceled: true };
        }

        try {
          await fs.promises.writeFile(
            result.filePath,
            JSON.stringify(playlist, null, 2),
          );

          return { canceled: false, filePath: result.filePath };
        } catch (e) {
          createLogger("main:scenes").error(
            "could not export the Scenes playlist",
            e,
          );

          return { canceled: false, error: e.message };
        }
      });

      ipcMain.handle("import-scenes-playlist", async () => {
        const result = await dialog.showOpenDialog(window, {
          title: "Import Scenes Playlist",
          filters: [{ name: "Scenes Playlist", extensions: ["json"] }],
          properties: ["openFile"],
          multiSelections: false,
        });

        if (result.canceled || !result.filePaths.length) {
          return { canceled: true };
        }

        try {
          const fileBuffer = await fs.promises.readFile(result.filePaths[0]);
          const playlist = JSON.parse(fileBuffer.toString());

          return {
            canceled: false,
            filePath: result.filePaths[0],
            playlist,
          };
        } catch (e) {
          createLogger("main:scenes").error(
            "could not import the Scenes playlist",
            e,
          );

          return { canceled: false, error: e.message };
        }
      });

      window.on("close", (e) => {
        if (isQuitting()) {
          return;
        }

        // Block the close synchronously, before any await. The async path
        // either leaves the window open (No) or quits the app explicitly.
        e.preventDefault();

        if (mainWindowClosing) {
          return;
        }

        mainWindowClosing = true;

        void closeMainWindow(window);
      });

      // Check for updates
      // autoUpdater.checkForUpdatesAndNotify();

      if (process.platform !== "linux") {
        await checkMediaPermission();
      }
    },

    destroy() {
      ipcMain.removeAllListeners("open-window");
      ipcMain.removeAllListeners("modv-ready");
      ipcMain.removeAllListeners("modv-destroy");
      ipcMain.removeAllListeners("get-media-manager-state");
      ipcMain.removeAllListeners("save-file");
      ipcMain.removeAllListeners("delete-scene");
      ipcMain.removeAllListeners("current-project");
      ipcMain.removeAllListeners("input-update");
      ipcMain.removeAllListeners("log");
      ipcMain.removeHandler("is-modv-ready");
      ipcMain.removeHandler("get-logs-folder");
      ipcMain.removeHandler("open-logs-folder");
      ipcMain.removeHandler("export-scenes-playlist");
      ipcMain.removeHandler("import-scenes-playlist");
    },
  },

  splashScreen: {
    devPath: "/splash-screen.html",
    prodPath: "splash-screen.html",
    options: {
      show: !isDevelopment,
      webPreferences: {
        nodeIntegration: true,
      },
      transparent: true,
      frame: false,
      resizable: false,
      skipTaskbar: true,
      fullscreenable: false,
      center: true,
      movable: false,
      backgroundColor: "#00000000",
      hasShadow: false,
      width: 600,
      height: 600,
    },
    unique: true,

    async create(window) {
      ipcMain.on("modv-ready", () => {
        try {
          window.close();
        } catch (e) {
          console.error(e);
        }

        placeMainWindow(windows["mainWindow"]);
      });
    },
  },
};

export { windowPrefs };
