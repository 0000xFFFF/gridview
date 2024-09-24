// preload.js
const { contextBridge, ipcRenderer } = require('electron');
const fs = require('fs');

contextBridge.exposeInMainWorld('ipcRenderer', ipcRenderer);
contextBridge.exposeInMainWorld('fs', fs);
contextBridge.exposeInMainWorld('electronAPI', {
    onSelectedDirectory: (callback) => ipcRenderer.on('selected-directory', callback),
    selectFile: (filePath) => ipcRenderer.on('select-file', filePath),
    openFile: (filePath) => ipcRenderer.on('open-file', filePath),
})


process.once('loaded', () => {
    global.electron = require('electron')
    electron.webFrame.setZoomFactor(1)
})
