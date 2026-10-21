import {
  MessageChannelMain,
  app,
  ipcMain,
  net,
  protocol,
  session,
  desktopCapturer,
} from "electron";
import { APP_SCHEME } from "./background-constants";
import { createLogger, getLogDirectory } from "./logger";
import { openFile } from "./open-file";
import { registerSystemAudioIpc } from "./system-audio";
import { createWindow } from "./windows";

require("@electron/remote/main").initialize();

// The package name is "modv-vite", which would otherwise make dev builds use
// ~/.config/modv-vite. Packaged builds get "modV" from productName, so set it
// explicitly here to share one userData directory (and its media tree).
app.setName("modV");

const logger = createLogger("main");

logger.info("modV starting", {
  version: app.getVersion(),
  platform: process.platform,
  logDirectory: getLogDirectory(),
});

// modV's shader, gallery-shader, ISF, gallery-ISF and three renderers already
// hold five long-lived WebGL contexts. The old cap of 6 left no room for the
// Point10 scenes or the transition shader, and Chromium silently force-lost
// the ISF contexts - imported layers rendered but their effects stopped
// applying. Point10 keeps at most four live scene contexts (see
// application/point10/adapter.js), so 16 leaves headroom for the layers, the
// gallery preview and a transition.
app.commandLine.appendSwitch("max-active-webgl-contexts", 16);
app.commandLine.appendSwitch("disable-backgrounding-occluded-windows", "true");
app.commandLine.appendSwitch("disable-background-timer-throttling", "true");
app.commandLine.appendSwitch("disable-renderer-backgrounding", "true");
app.commandLine.appendSwitch("disable-features", "CalculateNativeWinOcclusion");

// Point10's WebGPU scenes (three.js TSL) need a real adapter, not the
// software fallback. Vulkan is the backend that works on Linux without the
// SwiftShader path Electron uses by default.
//
// The loopback features let getDisplayMedia({ audio: true }) return the
// desktop audio on macOS. Electron 31 hides that behind feature flags and
// has no working equivalent on Linux, where the desktop audio is captured
// from a PulseAudio/PipeWire monitor source instead.
app.commandLine.appendSwitch(
  "enable-features",
  "Vulkan,WebGPU,MacLoopbackAudioForScreenShare,MacSckSystemAudioLoopbackOverride",
);
app.commandLine.appendSwitch("enable-unsafe-webgpu");

// Lets PulseAudio/PipeWire monitor sources be listed and captured as input
// devices, which is how the system audio is captured on Linux.
app.commandLine.appendSwitch("allow-monitor-inputs", "true");

const isDevelopment = process.env.NODE_ENV !== "production";

// Scheme must be registered before the app is ready
protocol.registerSchemesAsPrivileged([
  {
    scheme: APP_SCHEME,
    privileges: {
      secure: true,
      standard: true,
      stream: true,
      supportFetchAPI: true,
      bypassCSP: true,
    },
  },
]);

app.on("open-file", (event, filePath) => {
  event.preventDefault();

  openFile(filePath);
});

// Log crash details so e2e runs (and bug reports) can distinguish an app
// crash from a test-level failure.
app.on("render-process-gone", (event, webContents, details) => {
  logger.error("renderer process gone", {
    reason: details.reason,
    exitCode: details.exitCode,
    url: webContents.getURL(),
  });
});

app.on("child-process-gone", (event, details) => {
  logger.error("child process gone", {
    type: details.type,
    reason: details.reason,
    exitCode: details.exitCode,
    name: details.name || "",
  });
});

process.on("uncaughtException", (error) => {
  logger.error("uncaught exception in the main process", error);
});

process.on("unhandledRejection", (reason) => {
  logger.error("unhandled rejection in the main process", reason);
});

// Quit when all windows are closed.
app.on("window-all-closed", () => {
  // On macOS it is common for applications and their menu bar
  // to stay active until the user quits explicitly with Cmd + Q
  if (process.platform !== "darwin") {
    app.quit();
  }
});

app.on("activate", async () => {
  // On macOS it's common to re-create a window in the app when the
  // dock icon is clicked and there are no other windows open.
  createWindow({ windowName: "mainWindow" });
});

function setupColorPickerUIMessageChannels(mainWindow, colorPickerWindow) {
  const {
    port1: colorPickerUIChannel_mainWindowPort,
    port2: colorPickerUIChannel_colorPickerPort,
  } = new MessageChannelMain();

  mainWindow.webContents.postMessage("port:colorPickerUI", null, [
    colorPickerUIChannel_mainWindowPort,
  ]);

  colorPickerWindow.webContents.postMessage("port:colorPickerUI", null, [
    colorPickerUIChannel_colorPickerPort,
  ]);

  // return {
  //   colorPickerUIChannel_mainWindowPort: port1,
  //   colorPickerUIChannel_colorPickerPort: port2,
  // };
}

// This method will be called when Electron has finished
// initialization and is ready to create browser windows.
// Some APIs can only be used after this event occurs.
app.on("ready", async () => {
  registerSystemAudioIpc();

  protocol.handle(APP_SCHEME, (request) => {
    const newPath =
      "file://" +
      new URL("http://modv.com/" + request.url.slice(`${APP_SCHEME}://`.length))
        .pathname;

    return net.fetch(newPath);
  });

  app.commandLine.appendSwitch(
    "disable-backgrounding-occluded-windows",
    "true",
  );

  // Use the native/system picker if available for getDisplayMedia; otherwise
  // provide sources via desktopCapturer (screen + window)
  try {
    session.defaultSession.setDisplayMediaRequestHandler(
      async (request, callback) => {
        const displayMediaLogger = createLogger("main:display-media");

        displayMediaLogger.info("display media request received", {
          video: request?.video ? { id: request.video.id } : null,
          audio: request?.audio ?? null,
        });

        try {
          const sources = await desktopCapturer.getSources({
            types: ["screen", "window"],
            thumbnailSize: { width: 0, height: 0 },
          });

          displayMediaLogger.info(
            "desktop capturer returned sources",
            sources.map((source) => ({
              id: source.id,
              name: source.name,
              display_id: source.display_id,
            })),
          );

          // Send the list to the renderer for UI selection if desired.
          // For now, grant the first source matching the requested surface type.
          let chosen = sources[0];
          if (request && request.video && request.video.id) {
            const match = sources.find((s) => s.id === request.video.id);
            if (match) chosen = match;
          }

          if (!chosen) {
            displayMediaLogger.error(
              "no desktop source was available for the request",
            );
            callback({});
            return;
          }

          // Electron's desktop audio loopback is only reliable on macOS and
          // Windows. On Linux it hands back a stream without an audio track
          // and can take the browser process down with it, so the system audio
          // is captured from a PulseAudio/PipeWire monitor source instead.
          const wantsLoopback =
            process.platform === "darwin" || process.platform === "win32";

          const granted = {
            video: { source: chosen, frameRate: 60 },
          };

          if (wantsLoopback) {
            granted.audio = "loopback";
          }

          displayMediaLogger.info("granting display media request", {
            source: chosen.name,
            loopbackAudio: wantsLoopback,
          });

          callback(granted);
        } catch (e) {
          displayMediaLogger.error("setDisplayMediaRequestHandler error", e);
          callback({});
        }
      },
      { useSystemPicker: true },
    );
  } catch (e) {
    logger.warn("DisplayMediaRequestHandler not available", {
      message: e?.message,
    });
  }

  const mainWindow = createWindow({ windowName: "mainWindow" });
  ipcMain.once("main-window-created", () => {
    createWindow({ windowName: "splashScreen" });

    /////// color picker

    const {
      port1: colorPickerMainChannel_mainPort,
      port2: colorPickerMainChannel_colorPickerPort,
    } = new MessageChannelMain();

    colorPickerMainChannel_mainPort.on("message", (e) => {
      if (e.data === "create-port:colorPickerUI") {
        setupColorPickerUIMessageChannels(mainWindow, colorPickerWindow);
      }
    });

    const colorPickerWindow = createWindow({
      windowName: "colorPicker",
      options: { show: false },
    });

    colorPickerWindow.once("ready-to-show", () => {
      setupColorPickerUIMessageChannels(mainWindow, colorPickerWindow);

      colorPickerWindow.webContents.postMessage("port:colorPickerMain", null, [
        colorPickerMainChannel_colorPickerPort,
      ]);
    });
  });
});

// Exit cleanly on request from parent process in development mode.
if (isDevelopment) {
  if (process.platform === "win32") {
    process.on("message", (data) => {
      if (data === "graceful-exit") {
        app.quit();
      }
    });
  } else {
    process.on("SIGTERM", () => {
      app.quit();
    });
  }
}
