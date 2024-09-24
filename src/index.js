const { app, BrowserWindow, ipcMain, dialog, Menu, globalShortcut, nativeImage } = require('electron');
const fs = require('fs');
const path = require('path');
const { exec } = require('child_process');
const os = require('os');

let win;

function createWindow() {
    win = new BrowserWindow({
        width: 800,
        height: 600,
        webPreferences: {
            nodeIntegration: true,
            contextIsolation: false,
            zoomFactor: 1,
            preload: path.resolve(path.join('src', 'preload.js'))
        },
        icon: path.join(app.getAppPath(), 'assets', 'icon.png') // Set application icon
    });

    win.webContents.setZoomFactor(1); 

    const startUrl = path.join(app.getAppPath(), 'src', 'index.html');
    win.loadFile(startUrl);

    const menu = Menu.buildFromTemplate([
        {
            label: 'File',
            submenu: [
                {
                    label: 'Select Directory',
                    click: async () => {
                        const selectedDir = await selectDirectory();
                        if (selectedDir) {
                            win.webContents.send('selected-directory', selectedDir);  // Send selected directory to renderer
                        }
                    }
                },
                { type: 'separator' },
                {
                    label: 'Show Dev Tools',
                    click: () => {
                        win.webContents.openDevTools(); // Opens the DevTools for the current window
                    }
                },
                { type: 'separator' },
                {
                    label: 'Quit',
                    role: 'quit'
                }
            ]
        }
    ]);


    Menu.setApplicationMenu(menu);

    // Register the shortcut for opening Developer Tools
    globalShortcut.register('Control+Shift+I', () => { win.webContents.openDevTools(); });
}

app.whenReady().then(createWindow);


async function selectDirectory() {
    const result = await dialog.showOpenDialog(win, { properties: ['openDirectory'] });
    if (result.canceled) { return null; } // No directory was chosen

    const dirPath = result.filePaths[0];
    //console.log(`selected dir: ${dirPath}`);
    win.setTitle(`GridView - ${dirPath}`);
    return getMediaDirectories(dirPath); // Return media files from the selected directory
}

// Function to retrieve directories and files
async function getMediaDirectories(dirPath) {
    return new Promise((resolve, reject) => {
        fs.readdir(dirPath, { withFileTypes: true }, async (err, files) => {
            if (err) return reject(err);

            const directories = [];
            const mediaFiles = await getMediaFiles(dirPath); // Get files in the root directory

            // Only add the root directory if it has media files
            if (mediaFiles.length > 0) {
                directories.push({
                    path: '.', // Represent the root directory as '.'
                    files: mediaFiles
                });
            }

            // Check subdirectories
            for (const file of files) {
                const fullPath = path.join(dirPath, file.name);

                if (file.isDirectory()) {
                    const subMediaFiles = await getMediaFiles(fullPath); // Get media files in the subdirectory
                    if (subMediaFiles.length > 0) { // Only add if not empty
                        directories.push({
                            path: file.name,
                            files: subMediaFiles
                        });
                    }
                }
            }

            resolve(directories);
        });
    });
}


function getMediaFiles(dirPath) {
    return new Promise((resolve, reject) => {
        const mediaExtensions = ['.png', '.jpg', '.jpeg', '.gif', '.mp4', '.webm', '.mov', '.avi'];
        const mediaFiles = [];

        fs.readdir(dirPath, (err, files) => {
            if (err) return reject(err);

            files.forEach(file => {
                const fullPath = path.join(dirPath, file);
                const extname = path.extname(file).toLowerCase();

                // Check if the file has a media extension
                if (mediaExtensions.includes(extname)) {
                    //console.log(fullPath);
                    mediaFiles.push({
                        name: file,
                        path: fullPath,  // Full path to the file
                    });
                }
            });

            resolve(mediaFiles); // Resolve the promise with media files
        });
    });
}


function selectFile(filePath) {
    const absolutePath = path.resolve(filePath);
    if      (os.platform() === 'win32') { exec(`explorer /select, "${absolutePath.replace(/\//g, '\\')}"`); }
    else if (os.platform() === 'darwin') { exec(`open -R "${absolutePath}"`); }
    else {
        //exec(`xdg-open "${path.dirname(absolutePath)}"`);
        exec(`dolphin --select "${absolutePath}"`);
    }
}

function openFile(filePath) {
    const absolutePath = path.resolve(filePath);
    if      (os.platform() === 'win32')  { exec(`start "" "${absolutePath.replace(/\//g, '\\')}"`); }
    else if (os.platform() === 'darwin') { exec(`open "${absolutePath}"`); }
    else                                 { exec(`xdg-open "${absolutePath}"`); }
}

// IPC listeners
ipcMain.on('select-file', (event, filePath) => { selectFile(filePath); });
ipcMain.on('open-file', (event, filePath) => { openFile(filePath); });


app.on('window-all-closed', () => {
    if (process.platform !== 'darwin') app.quit();
});
