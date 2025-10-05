const img_popup = document.createElement("img");
img_popup.className = "media-file-popup";
document.body.appendChild(img_popup);

function addFileInfo(div_file, file) {
    const div_file_info = document.createElement("span");
    div_file_info.className = "media-file-info";
    const div_file_info_name = document.createElement("a");
    div_file_info_name.textContent = file.name;
    div_file_info_name.className = "media-file-info-name";
    div_file_info_name.addEventListener("mouseup", (event) => {
        switch (event.button) {
            case 1:
                ipcRenderer.send("open-file", file.path);
                break;
            case 2:
                ipcRenderer.send("select-file", file.path);
                break;
        }
    });
    div_file_info.appendChild(div_file_info_name);
    div_file.addEventListener("mouseover", function () {
        div_file_info.style.display = "block";
        div_file_info.style.opacity = "1"; // Make it visible
    });
    div_file.addEventListener("mouseleave", function () {
        div_file_info.style.opacity = "0"; // Hide with transition
        setTimeout(() => {
            div_file_info.style.display = "none"; // Hide after transition
        }, 300); // Match this to the duration of the CSS transition
    });

    div_file.appendChild(div_file_info);
    return div_file_info;
}

const CONCURRENT_LOADS = 3;
const LOAD_DELAY = 10;
const PRELOAD_VIEWPORT_BUFFER = 200;
let loadQueue = [];
let activeLoads = 0;
let preloadCache = new Map();

function delay(ms) {
    return new Promise((resolve) => setTimeout(resolve, ms));
}

function isNearViewport(element, buffer = PRELOAD_VIEWPORT_BUFFER) {
    const rect = element.getBoundingClientRect();
    return (
        rect.bottom >= -buffer &&
        rect.top <= window.innerHeight + buffer &&
        rect.right >= -buffer &&
        rect.left <= window.innerWidth + buffer
    );
}

async function preloadImage(url, priority = "low") {
    if (preloadCache.has(url)) {
        return preloadCache.get(url);
    }

    return new Promise((resolve, reject) => {
        loadQueue.push({ url, resolve, reject, priority });
        processLoadQueue();
    });
}

async function processLoadQueue() {
    if (activeLoads >= CONCURRENT_LOADS || loadQueue.length === 0) return;

    // Sort by priority
    loadQueue.sort((a, b) => {
        const priorities = { high: 3, medium: 2, low: 1 };
        return priorities[b.priority] - priorities[a.priority];
    });

    const item = loadQueue.shift();
    activeLoads++;

    try {
        await new Promise((resolve) => setTimeout(resolve, LOAD_DELAY));
        preloadCache.set(item.url, true);
        item.resolve();
    } catch (error) {
        item.reject(error);
    } finally {
        activeLoads--;
        setTimeout(processLoadQueue, 50);
    }
}

function addChildImage(div_file, file, div_file_info) {
    const img = document.createElement("img");
    img.loading = "lazy";
    img.decoding = "async";
    img.className = "media-file-img";
    img.dataset.filepath = file.path;

    // Set placeholder
    img.style.backgroundColor = "#1a1a1a";
    img.style.minHeight = "100px";

    // Add hover priority handling
    div_file.addEventListener("mouseenter", () => {
        if (!img.src || img.src === img.dataset.placeholder) {
            const url = `file://${file.path}`;
            const item = loadQueue.find((i) => i.url === url);
            if (item) {
                item.priority = "high";
                // Force reprocess queue with new priority
                processLoadQueue();
            } else if (!preloadCache.has(url)) {
                // Start loading if not already loaded
                loadImage("high");
            }
        }
    });

    const loadImage = async (priority = "low") => {
        if (!isNearViewport(div_file)) return;
        try {
            const url = `file://${file.path}`;

            // Check cache first
            if (preloadCache.has(url)) {
                img.src = url;
                return;
            }

            // Add to load queue
            await new Promise((resolve, reject) => {
                loadQueue.push({ url, resolve, reject, priority });
                processLoadQueue();
            });

            img.src = url;
        } catch (err) {
            console.error("Failed to load image:", err);
        }
    };

    // Use Intersection Observer for lazy loading
    const observer = new IntersectionObserver(
        (entries) => {
            entries.forEach((entry) => {
                if (entry.isIntersecting) {
                    loadImage(isNearViewport(div_file) ? "medium" : "low");
                    observer.unobserve(img);
                }
            });
        },
        { rootMargin: "200px" }
    );

    observer.observe(img);
    div_file.appendChild(img);
    return Promise.resolve();
}

function throttle(fn, delay) {
    let lastCall = 0;
    return function (...args) {
        const now = Date.now();
        if (now - lastCall >= delay) {
            fn.apply(this, args);
            lastCall = now;
        }
    };
}

function isElementInViewport(el) {
    const rect = el.getBoundingClientRect();
    return (
        rect.top >= -rect.height &&
        rect.left >= -rect.width &&
        rect.top <=
            (window.innerHeight || document.documentElement.clientHeight) +
                rect.height &&
        rect.left <=
            (window.innerWidth || document.documentElement.clientWidth) +
                rect.width
    );
}

function addChildVideo(div_file, file, div_file_info) {
    return new Promise((resolve) => {
        // Make container relative for overlay
        div_file.style.position = "relative";

        // Thumbnail image
        const thumbImg = document.createElement("img");
        thumbImg.src = `file://${file.thumb || file.path}`;
        thumbImg.className = "video-thumb";
        div_file.appendChild(thumbImg);

        // Play button overlay
        const playBtn = document.createElement("div");
        playBtn.className = "play-button";
        playBtn.innerHTML = "&#9658;"; // Triangle play symbol
        div_file.appendChild(playBtn);

        // On click, replace thumbnail with actual video
        thumbImg.addEventListener("click", () => {
            const video = document.createElement("video");
            video.src = `file://${file.path}`;
            video.controls = true;
            video.autoplay = false;
            video.loop = false;
            video.muted = true;
            div_file.innerHTML = ""; // Clear thumbnail + button
            div_file.appendChild(video);
        });

        resolve();
    });
}

function updatePriorities() {
    document.querySelectorAll(".media-file-img").forEach((img) => {
        if (!img.src || img.src === img.dataset.placeholder) {
            const url = `file://${img.closest(".media-file").dataset.filepath}`;
            const item = loadQueue.find((i) => i.url === url);
            if (item) {
                item.priority = isNearViewport(img) ? "high" : "low";
            }
        }
    });
    // Re-sort queue based on new priorities
    processLoadQueue();
}

// Add throttled priority updates
const throttledUpdatePriorities = throttle(updatePriorities, 150);

// Add scroll and resize listeners for priority updates
window.addEventListener("scroll", throttledUpdatePriorities);
window.addEventListener("resize", throttledUpdatePriorities);

function createFile(file) {
    const div_file = document.createElement("div");
    div_file.className = "media-file";
    div_file.dataset.filepath = file.path; // Store filepath for priority updates

    // File info
    let div_file_info = addFileInfo(div_file, file);

    // Image or video
    if (
        file.name.endsWith(".png") ||
        file.name.endsWith(".jpg") ||
        file.name.endsWith(".jpeg") ||
        file.name.endsWith(".gif")
    ) {
        return addChildImage(div_file, file, div_file_info).then(
            () => div_file
        ); // Return a promise that resolves to the div_file
    } else if (
        file.name.endsWith(".mp4") ||
        file.name.endsWith(".webm") ||
        file.name.endsWith(".mov") ||
        file.name.endsWith(".avi")
    ) {
        return addChildVideo(div_file, file, div_file_info).then(
            () => div_file
        ); // Return a promise that resolves to the div_file
    }
    return Promise.resolve(div_file); // Return the div_file if no async media loading is required
}

window.electronAPI.onSelectedDirectory(async (event, directories) => {
    const div_dirs = document.getElementById("media-dirs");
    div_dirs.innerHTML = ""; // Clear existing content

    const topbar = document.getElementById("topbar");

    // Show loading GIF in topbar
    const loadingGif = document.createElement("img");
    loadingGif.src = "../assets/load.gif"; // Replace with the actual path to your GIF
    loadingGif.className = "loading-gif"; // Optional: Add a class for styling
    topbar.appendChild(loadingGif); // Add the loading GIF to the topbar

    const loadPromises = []; // Array to store image/video loading promises

    for (const dir of directories) {
        const div_dir = document.createElement("div");
        div_dir.className = "media-dir";

        const div_dir_head = document.createElement("div");
        div_dir_head.className = "media-dir-head";
        const h1 = document.createElement("h1");
        h1.textContent = dir.path;
        div_dir_head.appendChild(h1);
        div_dir.appendChild(div_dir_head);

        const div_dir_files = document.createElement("div");
        div_dir_files.className = "media-dir-files";
        div_dir_files.style.columnCount = setting_cols;

        for (const file of dir.files) {
            // Create the file element and append it immediately
            const filePromise = createFile(file).then((div_file) => {
                div_dir_files.appendChild(div_file);
            });
            loadPromises.push(filePromise); // Add the promise to the loadPromises array
        }

        div_dir.append(div_dir_files);
        div_dirs.append(div_dir);
    }

    // Wait for all images/videos to load
    try {
        await Promise.all(loadPromises);
    } catch (err) {
        console.error("Error loading some files:", err);
    }

    // Remove the loading GIF after all images/videos have loaded
    topbar.removeChild(loadingGif);
});
