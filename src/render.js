const img_popup = document.createElement("img");
img_popup.className = "media-file-popup";
document.body.appendChild(img_popup);

function isElementInViewport(el) {
    const rect = el.getBoundingClientRect();
    return (
        rect.top >= 0 &&
        rect.left >= 0 &&
        rect.bottom <=
        (window.innerHeight || document.documentElement.clientHeight) &&
        rect.right <=
        (window.innerWidth || document.documentElement.clientWidth)
    );
}

function isNearViewport(el) {
    const rect = el.getBoundingClientRect();
    const buffer = 1080 * 2; // pixels
    return (
        rect.top <
        (window.innerHeight || document.documentElement.clientHeight) +
        buffer &&
        rect.bottom > -buffer &&
        rect.left <
        (window.innerWidth || document.documentElement.clientWidth) +
        buffer &&
        rect.right > -buffer
    );
}

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
    div_file.addEventListener("mouseover", function() {
        div_file_info.style.display = "block";
        div_file_info.style.opacity = "1"; // Make it visible
        document.title = `GridView - ${file.name}`;
    });
    div_file.addEventListener("mouseleave", function() {
        div_file_info.style.opacity = "0"; // Hide with transition
        setTimeout(() => {
            div_file_info.style.display = "none"; // Hide after transition
        }, 300); // Match this to the duration of the CSS transition

        document.title = `GridView`;
    });

    div_file.appendChild(div_file_info);
    return div_file_info;
}

// Intersection Observer for lazy loading video thumbnails
const videoObserver = new IntersectionObserver(
    (entries) => {
        entries.forEach((entry) => {
            if (isNearViewport(entry.target)) {
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

function addChildImage(div_file, file, div_file_info) {
    const img = document.createElement("img");
    img.src = file.path;
    img.loading = "eager";
    img.decoding = "async";
    img.className = "media-file-img";
    img.dataset.filepath = file.path;

    // Set placeholder
    img.style.backgroundColor = "#1a1a1a";
    img.style.minHeight = "100px";

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

    if (currentDirIndex >= allDirectories.length) { return; }

    isLoadingMore = true;

    // Render current directory
    renderDirectory(
        allDirectories[currentDirIndex],
        currentDirIndex,
        allDirectories.length
    );

    currentDirIndex++;

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

    for (const file of dir.files) {
        createFile(file).then((div_file) => {
            div_dir_files.appendChild(div_file);
        });
    }

    div_dir.appendChild(div_dir_files);
    const mediaDirs = document.getElementById("media-dirs");
    mediaDirs.insertBefore(div_dir, footer);
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

    if (isNearViewport(footer)) {
        loadNextDirectory();
    }
}

setInterval(() => {
    loadMore();
}, 500);

// Hover preview functionality
function setupHoverPreview(
    mediaWrapper,
    mediaData,
    thumbnailImg,
    isVideo = false
) {
    let previewOverlay = null;
    let lastVolume = 1.0;

    const zoomCb = document.getElementById("setting_cb_hoverZoom");

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

    const setting_cb_hoverZoom = document.getElementById(
        "setting_cb_hoverZoom"
    );

    const showPreview = (e) => {
        document
            .querySelectorAll(".fcm_hover_preview")
            .forEach((el) => el.remove());

        if (!previewOverlay && setting_cb_hoverZoom.checked) {
            previewOverlay = document.createElement("div");
            previewOverlay.className = "fcm_hover_preview";

            if (isVideo) {
                const mainVideo = mediaWrapper.querySelector("video");
                const previewVideo = document.createElement("video");
                previewVideo.src = `file://${mediaData.path}`;
                previewVideo.loop = true;
                previewVideo.playsInline = true;
                previewVideo.controls = false;
                previewVideo.autoplay = true;
                previewVideo.volume = lastVolume;

                previewVideo.addEventListener("loadeddata", (event) => { updatePreviewPosition(e); });

                // Mouse wheel volume control
                mediaWrapper.addEventListener(
                    "wheel",
                    (e) => {
                        if (zoomCb.checked) {
                            e.preventDefault();
                            const delta = -e.deltaY * 0.0005;
                            const newVolume = Math.min(1, Math.max(0, previewVideo.volume + delta));
                            previewVideo.volume = newVolume;
                            lastVolume = newVolume;
                        }
                    },
                    { passive: false } // Important to make preventDefault() work
                );

                // Sync with main video if it exists and is playing
                if (mainVideo) {
                    previewVideo.currentTime = mainVideo.currentTime;
                    previewVideo.muted = false;
                    mainVideo.muted = true;
                } else {
                    previewVideo.muted = false;
                }

                previewOverlay.appendChild(previewVideo);
            } else {
                const previewImg = document.createElement("img");
                previewImg.src = `file://${mediaData.path}`;
                previewOverlay.appendChild(previewImg);
                previewImg.addEventListener("loadeddata", (event) => { updatePreviewPosition(e); });
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
        }
    };

    mediaWrapper.addEventListener("mouseenter", showPreview);
    mediaWrapper.addEventListener("mousemove", updatePreviewPosition);
    mediaWrapper.addEventListener("mouseleave", hidePreview);
}
