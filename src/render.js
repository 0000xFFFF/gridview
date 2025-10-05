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
        document.title = `GridView - ${file.name}`;
    });
    div_file.addEventListener("mouseleave", function () {
        div_file_info.style.opacity = "0"; // Hide with transition
        setTimeout(() => {
            div_file_info.style.display = "none"; // Hide after transition
        }, 300); // Match this to the duration of the CSS transition

        document.title = `GridView`;
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

async function preloadMedia(url, element, priority = "low") {
    if (preloadCache.has(url)) {
        return preloadCache.get(url);
    }

    return new Promise((resolve, reject) => {
        loadQueue.push({ url, resolve, reject, priority, element: element });
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
        setTimeout(processLoadQueue, 10);
    }
}

function updateQueuePriority(url, newPriority) {
    const item = loadQueue.find((i) => i.url === url);
    if (item) {
        item.priority = newPriority;
    }
}

function updatePriorities() {
    loadQueue.forEach((item) => {
        if (isNearViewport(item.element)) {
            item.priority = "medium";
        }
    });
    processLoadQueue();
}

// Intersection Observer for lazy loading
const imageObserver = new IntersectionObserver(
    (entries) => {
        entries.forEach((entry) => {
            if (entry.isIntersecting) {
                const img = entry.target;
                const fullUrl = img.dataset.filepath;

                if (fullUrl && !img.dataset.loading) {
                    img.dataset.loading = "true";

                    preloadMedia(
                        fullUrl,
                        img,
                        isNearViewport(img) ? "medium" : "low"
                    )
                        .then((fullImg) => {
                            setTimeout(() => {
                                img.src = fullUrl;
                                img.removeAttribute("data-full-url");
                                img.removeAttribute("data-loading");
                            }, 200);
                        })
                        .catch(() => {
                            img.removeAttribute("data-loading");
                        });
                }
            }
        });
    },
    {
        rootMargin: "200px", // Start loading 200px before entering viewport
    }
);

// Intersection Observer for lazy loading video thumbnails
const videoObserver = new IntersectionObserver(
    (entries) => {
        entries.forEach((entry) => {
            if (entry.isIntersecting) {
                const thumbImg = entry.target;
                const videoPath = thumbImg.dataset.videopath;

                if (videoPath && !thumbImg.dataset.loading) {
                    thumbImg.dataset.loading = "true";
                    window.electronAPI
                        .generateVideoThumbnail(videoPath)
                        .then((thumbPath) => {
                            if (thumbPath) {
                                thumbImg.src = `file://${thumbPath}`;
                            } else {
                                // Maybe set a "no thumbnail" image
                            }
                            thumbImg.removeAttribute("data-loading");
                            thumbImg.removeAttribute("data-videopath");
                        })
                        .catch((err) => {
                            console.error("Thumbnail generation failed:", err);
                            thumbImg.removeAttribute("data-loading");
                        });
                    videoObserver.unobserve(thumbImg);
                }
            }
        });
    },
    {
        rootMargin: "200px",
    }
);

function throttle(fn, delay) {
    let lastCall = 0;
    let timeout;

    return function (...args) {
        const now = Date.now();

        if (now - lastCall < delay) {
            clearTimeout(timeout);
            timeout = setTimeout(() => {
                lastCall = Date.now();
                fn.apply(this, args);
            }, delay - (now - lastCall));
        } else {
            lastCall = now;
            fn.apply(this, args);
        }
    };
}

// Add throttled priority updates
const throttledUpdatePriorities = throttle(updatePriorities, 3000);

// Add scroll and resize listeners for priority updates
window.addEventListener("scroll", throttledUpdatePriorities);
window.addEventListener("resize", throttledUpdatePriorities);

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
    img.addEventListener("mouseenter", () => {
        updateQueuePriority(file.path, "high");
    });

    imageObserver.observe(img);
    div_file.appendChild(img);
    setupHoverPreview(div_file, file, img, false);
    return Promise.resolve();
}

function addChildVideo(div_file, file, div_file_info) {
    return new Promise((resolve) => {
        // Make container relative for overlay
        div_file.style.position = "relative";

        // Thumbnail image
        const thumbImg = document.createElement("img");
        thumbImg.className = "video-thumb";
        thumbImg.dataset.videopath = file.path;

        // Set placeholder style
        thumbImg.style.backgroundColor = "#1a1a1a";
        thumbImg.style.minHeight = "100px";

        videoObserver.observe(thumbImg);
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

        setupHoverPreview(div_file, file, thumbImg, true);

        resolve();
    });
}

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

let allDirectories = [];
let currentDirIndex = 0;
let isLoadingMore = false;
const footer = document.createElement("div");
const loadingGif = document.createElement("img");
loadingGif.src = "../assets/load.gif";
loadingGif.className = "loading-gif";
footer.appendChild(loadingGif);
footer.id = "infinite-scroll-footer";
footer.style.display = "none";
document.getElementById("media-dirs").appendChild(footer);

function loadNextDirectory() {
    if (currentDirIndex >= allDirectories.length) {
        footer.style.display = "none";
        return;
    }

    isLoadingMore = true;
    renderDirectory(
        allDirectories[currentDirIndex],
        currentDirIndex + 1,
        allDirectories.length
    );
    currentDirIndex++;
    // Allow a small delay before setting isLoadingMore to false to prevent rapid firing
    setTimeout(() => {
        isLoadingMore = false;
    }, 50);
}

function renderDirectory(dir, index, total) {
    const div_dir = document.createElement("div");
    div_dir.className = "media-dir";

    const div_dir_head = document.createElement("div");
    div_dir_head.className = "media-dir-head";
    const h1 = document.createElement("h1");
    h1.textContent = dir.path;
    div_dir_head.appendChild(h1);

    const dirInfo = document.createElement("span");
    dirInfo.className = "media-dir-info";
    dirInfo.textContent = `(${index}/${total}) - ${dir.files.length} files`;
    div_dir_head.appendChild(dirInfo);

    div_dir.appendChild(div_dir_head);

    const div_dir_files = document.createElement("div");
    div_dir_files.className = "media-dir-files";
    div_dir_files.style.columnCount = setting_cols;

    for (const file of dir.files) {
        createFile(file).then((div_file) => {
            div_dir_files.appendChild(div_file);
        });
    }

    div_dir.appendChild(div_dir_files);
    const mediaDirs = document.getElementById("media-dirs");
    mediaDirs.insertBefore(div_dir, footer); // Insert before the footer
}

// Replace the existing onSelectedDirectory handler with this one
window.electronAPI.onSelectedDirectory(async (event, directories) => {
    const div_dirs = document.getElementById("media-dirs");
    div_dirs.innerHTML = ""; // Clear existing content
    div_dirs.appendChild(footer); // Re-add footer after clearing

    allDirectories = directories;
    currentDirIndex = 0;
    isLoadingMore = false;

    // Load the first directory
    if (allDirectories.length > 0) {
        loadNextDirectory();
    }

    // Start observing the footer
    footer.style.display = "flex";
});

function loadMore() {
    if (isLoadingMore) return;

    if (isElementInViewport(footer)) {
        loadNextDirectory();
    }
}

window.addEventListener("scroll", loadMore);
window.addEventListener("resize", loadMore);

// Hover preview functionality
function setupHoverPreview(
    mediaWrapper,
    mediaData,
    thumbnailImg,
    isVideo = false
) {
    let previewOverlay = null;
    let lastVolume = 1.0;
    let mainVideo = null;

    const updatePreviewPosition = (e) => {
        if (!previewOverlay) return;

        const mediaElement = previewOverlay.querySelector("img, video");
        const infoElement = previewOverlay.querySelector(
            ".fcm_hover_preview_info"
        );

        if (mediaElement) {
            const rect = mediaElement.getBoundingClientRect();
            const buffer = 20;
            let left = e.clientX + buffer;
            let top = e.clientY + buffer;

            if (left + rect.width > window.innerWidth) {
                left = e.clientX - rect.width - buffer;
            }
            if (top + rect.height > window.innerHeight) {
                top = e.clientY - rect.height - buffer;
            }

            left = Math.max(buffer, left);
            top = Math.max(buffer, top);

            mediaElement.style.left = left + "px";
            mediaElement.style.top = top + "px";
        }

        if (infoElement) {
            infoElement.style.left = "50%";
            infoElement.style.bottom = "20px";
            infoElement.style.transform = "translateX(-50%)";
        }
    };

    const showPreview = (e) => {
        document
            .querySelectorAll(".fcm_hover_preview")
            .forEach((el) => el.remove());

        if (!previewOverlay && setting_hoverZoom) {
            previewOverlay = document.createElement("div");
            previewOverlay.className = "fcm_hover_preview";

            if (isVideo) {
                mainVideo = mediaWrapper.querySelector("video");
                const previewVideo = document.createElement("video");
                previewVideo.src = `file://${mediaData.path}`;
                previewVideo.loop = true;
                previewVideo.playsInline = true;
                previewVideo.controls = false;
                previewVideo.autoplay = true;
                previewVideo.volume = lastVolume;

                previewVideo.addEventListener("wheel", (e) => {
                    e.preventDefault();
                    const delta = e.deltaY * -0.01;
                    lastVolume = Math.max(
                        0,
                        Math.min(1, previewVideo.volume + delta)
                    );
                    previewVideo.volume = lastVolume;
                });

                if (mainVideo) {
                    previewVideo.currentTime = mainVideo.currentTime;
                    previewVideo.muted = false;
                    mainVideo.muted = true;
                }

                previewOverlay.appendChild(previewVideo);
            } else {
                const previewImg = document.createElement("img");
                previewImg.src = `file://${mediaData.path}`;
                previewOverlay.appendChild(previewImg);
            }

            // const previewInfo = document.createElement("div");
            // previewInfo.className = "fcm_hover_preview_info";
            // previewInfo.textContent = mediaData.name;
            // previewOverlay.appendChild(previewInfo);

            document.body.appendChild(previewOverlay);
        }
        if (previewOverlay) {
            previewOverlay.classList.add("active");
            updatePreviewPosition(e);
        }
    };

    const hidePreview = () => {
        if (previewOverlay) {
            previewOverlay.remove();
            previewOverlay = null;
            if (mainVideo) {
                mainVideo.muted = false;
            }
        }
    };

    mediaWrapper.addEventListener("mouseenter", showPreview);
    mediaWrapper.addEventListener("mousemove", updatePreviewPosition);
    mediaWrapper.addEventListener("mouseleave", hidePreview);
}
