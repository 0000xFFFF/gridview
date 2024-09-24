const { ipcRenderer } = require('electron');

let setting_hoverZoom = false;
let setting_cols = 6

function selectFile(filePath) { ipcRenderer.send('select-file', filePath); }
function openFile(filePath) { ipcRenderer.send('open-file', filePath); }

function topbar_setup() {

    let topbar = document.getElementById('topbar');

    // hide topbar on scroll
    let prevScrollPos = window.scrollY;
    window.onscroll = function () {
        const currentScrollPos = window.scrollY;
        topbar.style.top = prevScrollPos > currentScrollPos ? "0" : "-60px";
        prevScrollPos = currentScrollPos;
    };

    // setting: hover zoom
    let cb_hoverZoom = document.getElementById('setting_cb_hoverZoom');
    cb_hoverZoom.checked = setting_hoverZoom;
    cb_hoverZoom.addEventListener("change", function () { setting_hoverZoom = !setting_hoverZoom; });
    function hoverZoom_check() { setting_hoverZoom = cb_hoverZoom.checked = !cb_hoverZoom.checked; }

    // setting: media cols slider
    let slider = document.getElementById('setting_slider_cols');
    let slider_label = document.getElementById('setting_slider_cols_label');
    function slider_update() {
        slider_label.innerHTML = setting_cols = slider.value;
        let mediaDirFiles = document.querySelectorAll('.media-dir-files')
        mediaDirFiles.forEach(element => { element.style.columnCount = setting_cols; });
    }
    function slider_left() { slider.value = Math.max(Number(slider.value) - Number(slider.step), slider.min); slider_update(); }
    function slider_right() { slider.value = Math.min(Number(slider.value) + Number(slider.step), slider.max); slider_update(); }
    slider.value = setting_cols;
    slider.oninput = slider_update;
    slider.addEventListener('wheel', function (event) {
        if (event.deltaY < 0) { slider_left(); }
        else { slider_right(); }
        event.preventDefault();
    });
    slider_update();

    // setting up keyboard events for topbar
    document.addEventListener('keydown', (event) => {
        switch (event.key) {
            case 'p': hoverZoom_check(); break;
            case 'ArrowLeft': slider_left(); break;
            case 'ArrowRight': slider_right(); break;
        }
    });
}

topbar_setup();

const img_popup = document.createElement('img');
img_popup.className = 'media-file-popup';
document.body.appendChild(img_popup);

ipcRenderer.on('selected-directory', (event, directories) => {
    const div_dirs = document.getElementById('media-dirs');
    div_dirs.innerHTML = ''; // Clear existing content

    directories.forEach(dir => {
        const div_dir = document.createElement('div');
        div_dir.className = 'media-dir';

        const div_dir_head = document.createElement('div');
        div_dir_head.className = 'media-dir-head';
        const h1 = document.createElement('h1');
        h1.textContent = dir.path === '.' ? '/' : `${dir.path}`;
        div_dir_head.appendChild(h1);
        div_dir.appendChild(div_dir_head);

        const div_dir_files = document.createElement('div');
        div_dir_files.className = 'media-dir-files';

        dir.files.forEach(file => {
            const name = file['name'];
            const div_file = document.createElement('div');
            div_file.className = 'media-file';

            // file info
            const div_file_info = document.createElement('p');
            div_file_info.className = 'media-file-info';
            const div_file_info_name = document.createElement('a');
            div_file_info_name.textContent = `${file.name}`;
            div_file_info_name.className = 'media-file-info-name';
            div_file_info_name.addEventListener('mouseup', (event) => {
                switch (event.button) {
                    case 1: openFile(file.path);   break;
                    case 2: selectFile(file.path); break;
                }
            });
            div_file_info.appendChild(div_file_info_name);
            div_file.addEventListener('mouseover', function () {
                div_file_info.style.display = 'block';
                div_file_info.style.opacity = '1'; // Make it visible
            });
            div_file.addEventListener('mouseleave', function () {
                div_file_info.style.opacity = '0'; // Hide with transition
                setTimeout(() => {
                    div_file_info.style.display = 'none'; // Hide after transition
                }, 300); // Match this to the duration of the CSS transition
            });
            div_file.append(div_file_info);


            // Image or video
            if (name.endsWith('.png') || name.endsWith('.jpg') || name.endsWith('.jpeg') || name.endsWith('.gif')) {
                const img = document.createElement('img');
                img.src = `file://${file.path}`
                //img.loading = 'lazy';
                div_file.appendChild(img);

                img.addEventListener('load', () => {
                    const div_file_info_dims = document.createElement('p');
                    div_file_info_dims.className = 'media-file-info-dims';
                    div_file_info_dims.textContent = `${img.naturalWidth}x${img.naturalHeight}`;
                    div_file_info.appendChild(div_file_info_dims);
                });

                // image popup on hover
                div_file.addEventListener('mouseenter', function () {
                    if (!setting_hoverZoom) { return; }
                    img_popup.src = img.src;
                    img_popup.style.display = 'block';
                });
                div_file.addEventListener('mouseleave', function () {
                    img_popup.src = '';
                    img_popup.style.display = 'none';
                })
            } else if (name.endsWith('.mp4') || name.endsWith('.webm') || name.endsWith('.mov') || name.endsWith('.avi')) {
                const video = document.createElement('video');
                video.src = `file://${file.path}`
                video.controls = true;
                div_file.appendChild(video);
            }

            div_dir_files.appendChild(div_file);
        });

        div_dir.append(div_dir_files);
        div_dirs.append(div_dir);
    });
});
