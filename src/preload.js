// preload.js
const { contextBridge, ipcRenderer, webFrame } = require("electron");
const fs = require("fs");
const path = require("path");

contextBridge.exposeInMainWorld("fs", fs);
contextBridge.exposeInMainWorld("path", path);
contextBridge.exposeInMainWorld("electronAPI", {
    openFile: (filePath) => ipcRenderer.send("open-file", filePath),
    selectFile: (filePath) => ipcRenderer.send("select-file", filePath),
    dropFolder: (dirPath) => ipcRenderer.send("drop-folder", dirPath),
    onSelectedDirectory: (callback) =>
        ipcRenderer.on("selected-directory", callback),
    setZoomFactor: (factor) => webFrame.setZoomFactor(factor),
    reloadCurrentDirectory: () =>
        ipcRenderer.invoke("reload-current-directory"),
    generateVideoThumbnail: (videoPath) =>
        ipcRenderer.invoke("generate-video-thumbnail", videoPath),
    getThumbnailData: (thumbPath) =>
        ipcRenderer.invoke("get-thumbnail-data", thumbPath),
});

process.once("loaded", () => {
    global.electron = require("electron");
    electron.webFrame.setZoomFactor(1);
});
