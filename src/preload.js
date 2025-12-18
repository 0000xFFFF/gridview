// preload.js
const { contextBridge, ipcRenderer, webFrame } = require("electron");
const fs = require("fs");
const path = require("path");

contextBridge.exposeInMainWorld("ipcRenderer", ipcRenderer);
contextBridge.exposeInMainWorld("fs", fs);
contextBridge.exposeInMainWorld("path", path);
contextBridge.exposeInMainWorld("electronAPI", {
    onSelectedDirectory: (callback) =>
        ipcRenderer.on("selected-directory", callback),
    setZoomFactor: (factor) => webFrame.setZoomFactor(factor),
    generateVideoThumbnail: (videoPath) =>
        ipcRenderer.invoke("generate-video-thumbnail", videoPath),
    getThumbnailData: (thumbPath) =>
        ipcRenderer.invoke("get-thumbnail-data", thumbPath),
});

process.once("loaded", () => {
    global.electron = require("electron");
    electron.webFrame.setZoomFactor(1);
});
