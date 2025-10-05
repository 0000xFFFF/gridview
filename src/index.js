const {
    app,
    BrowserWindow,
    ipcMain,
    dialog,
    Menu,
    globalShortcut,
    nativeImage,
} = require("electron");
const fs = require("fs");
const path = require("path");
const { exec } = require("child_process");
const os = require("os");
const sizeOf = require("image-size").default || require("image-size");
const thumbDir = path.join(app.getPath("userData"), "thumbs");
let mainWindow;

app.whenReady().then(() => {
    // WINDOW SETTINGS
    mainWindow = new BrowserWindow({
        width: 800,
        height: 600,
        webPreferences: {
            nodeIntegration: true,
            audio: true,
            zoomFactor: 1,
            preload: path.resolve(
                path.join(app.getAppPath(), "src", "preload.js")
            ),
        },
        icon: path.join(app.getAppPath(), "assets", "icon.png"), // Set application icon
    });

    // LOAD INDEX
    const startUrl = path.join(app.getAppPath(), "src", "index.html");
    mainWindow.loadFile(startUrl);

    // MENU SETUP
    const menu = Menu.buildFromTemplate([
        {
            label: "File",
            submenu: [
                {
                    label: "Select Directory",
                    click: selectDirectoryAndSend,
                },
                { type: "separator" },
                {
                    label: "Show Dev Tools",
                    click: () => {
                        mainWindow.webContents.openDevTools(); // Opens the DevTools for the current window
                    },
                },
                { type: "separator" },
                {
                    label: "Quit",
                    role: "quit",
                },
            ],
        },
    ]);
    Menu.setApplicationMenu(menu);

    globalShortcut.register("Control+Shift+I", () => {
        mainWindow.webContents.openDevTools();
    });
});

fs.mkdirSync(thumbDir, { recursive: true });

// TODO: these need to be done async lazyly when needed
const LOAD_THUMBS = false;
const GET_DIMS = false;

async function generateVideoThumbnail(videoPath) {
    return new Promise((resolve, reject) => {
        if (!LOAD_THUMBS) {
            return resolve(null);
        }

        const thumbPath = path.join(
            thumbDir,
            path.basename(videoPath) + ".jpg"
        );

        // Skip if thumbnail already exists
        if (fs.existsSync(thumbPath)) {
            return resolve(thumbPath);
        }

        // Generate thumbnail with ffmpeg
        const cmd = `ffmpeg -i "${videoPath}" -ss 00:00:01 -vframes 1 -vf scale=160:-1 -q:v 31 "${thumbPath}" -y`;

        exec(cmd, (err) => {
            if (err) {
                console.error("Thumbnail generation failed:", err);
                resolve(null);
            } else {
                resolve(thumbPath);
            }
        });
    });
}

async function dims(imagePath) {
    if (!GET_DIMS) {
        return new Promise((resolve, reject) => {
            return resolve({ width: null, height: null });
        });
    }

    return new Promise((resolve, reject) => {
        fs.readFile(imagePath, (err, buffer) => {
            if (err) {
                console.error("Error reading file:", err);
                resolve({ width: null, height: null }); // Resolve with nulls to avoid breaking
                return;
            }

            try {
                const dimensions = sizeOf(buffer);
                resolve(dimensions);
            } catch (error) {
                console.error("Error getting dimensions:", error);
                resolve({ width: null, height: null });
            }
        });
    });
}

async function selectDirectoryAndSend() {
    const selectedDir = await selectDirectory();
    if (selectedDir) {
        mainWindow.webContents.send("selected-directory", selectedDir); // Send selected directory to renderer
    }
}

async function selectDirectory() {
    const result = await dialog.showOpenDialog(mainWindow, {
        properties: ["openDirectory"],
    });
    if (result.canceled) {
        return null;
    } // No directory was chosen

    const dirPath = result.filePaths[0];
    return await loadDir(dirPath);
}

async function loadDir(dirPath) {
    //console.log(`selected dir: ${dirPath}`);
    mainWindow.setTitle(`GridView - ${dirPath}`);
    return await getMediaDirectories(dirPath);
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
                    path: ".", // Represent the root directory as '.'
                    files: mediaFiles,
                });
            }

            // Check subdirectories recursively
            await processDirectories(files, dirPath, dirPath, directories);

            resolve(directories);
        });
    });
}

async function processDirectories(files, basePath, currentPath, directories) {
    for (const file of files) {
        const fullPath = path.join(currentPath, file.name);

        if (file.isDirectory()) {
            const subMediaFiles = await getMediaFiles(fullPath); // Get media files in the subdirectory
            if (subMediaFiles.length > 0) {
                // Only add if not empty
                directories.push({
                    path: fullPath.replace(`${basePath}/`, ""), // Use relative path
                    files: subMediaFiles,
                });
            }
            // Recursively process subdirectories
            const subFiles = await fs.promises.readdir(fullPath, {
                withFileTypes: true,
            });
            await processDirectories(subFiles, basePath, fullPath, directories); // Process subdirectory files
        }
    }
}

async function getMediaFiles(dirPath) {
    mainWindow.setTitle(`GridView - loading ${dirPath}`);
    const imageExtensions = [".png", ".jpg", ".jpeg", ".gif"];
    const videoExtensions = [".mp4", ".webm", ".mov", ".avi"];
    const mediaFiles = [];

    try {
        const files = await fs.promises.readdir(dirPath);

        for (const file of files) {
            const fullPath = path.join(dirPath, file);
            const extname = path.extname(file).toLowerCase();

            if (imageExtensions.includes(extname)) {
                mediaFiles.push({
                    name: file,
                    width: null,
                    height: null,
                    path: fullPath,
                    type: "image",
                });
            } else if (videoExtensions.includes(extname)) {
                const thumb = await generateVideoThumbnail(fullPath);
                mediaFiles.push({
                    name: file,
                    path: fullPath,
                    type: "video",
                    thumb: thumb,
                    width: null,
                    height: null,
                });
            }
        }
    } catch (err) {
        console.error("Error reading directory:", err);
    }

    mainWindow.setTitle(`GridView - loaded ${dirPath}`);
    return mediaFiles;
}

function selectFile(filePath) {
    const absolutePath = path.resolve(filePath);
    if (os.platform() === "win32") {
        exec(`explorer /select, "${absolutePath.replace(/\//g, "\\")}"`);
    } else if (os.platform() === "darwin") {
        exec(`open -R "${absolutePath}"`);
    } else {
        //exec(`xdg-open "${path.dirname(absolutePath)}"`);
        exec(`dolphin --select "${absolutePath}"`);
    }
}

function openFile(filePath) {
    const absolutePath = path.resolve(filePath);
    if (os.platform() === "win32") {
        exec(`start "" "${absolutePath.replace(/\//g, "\\")}"`);
    } else if (os.platform() === "darwin") {
        exec(`open "${absolutePath}"`);
    } else {
        exec(`xdg-open "${absolutePath}"`);
    }
}

// IPC listeners
ipcMain.on("select-file", (event, filePath) => {
    selectFile(filePath);
});
ipcMain.on("open-file", (event, filePath) => {
    openFile(filePath);
});
ipcMain.on("drop-folder", async (event, dirPath) => {
    const directories = await loadDir(dirPath);
    mainWindow.webContents.send("selected-directory", directories);
});

app.on("window-all-closed", () => {
    if (process.platform !== "darwin") app.quit();
});

function openFile(filePath) {
    const absolutePath = path.resolve(filePath);
    if (os.platform() === "win32") {
        exec(`start "" "${absolutePath.replace(/\//g, "\\")}"`);
    } else if (os.platform() === "darwin") {
        exec(`open "${absolutePath}"`);
    } else {
        exec(`xdg-open "${absolutePath}"`);
    }
}

// IPC listeners
ipcMain.on("select-file", (event, filePath) => {
    selectFile(filePath);
});
ipcMain.on("open-file", (event, filePath) => {
    openFile(filePath);
});
ipcMain.on("drop-folder", async (event, dirPath) => {
    const directories = await loadDir(dirPath);
    mainWindow.webContents.send("selected-directory", directories);
});

app.on("window-all-closed", () => {
    if (process.platform !== "darwin") app.quit();
});
