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

let currentZoomFactor = 1.0; // Initial zoom level

window.addEventListener('wheel', (event) => {
    if (event.ctrlKey && event.shiftKey) {
        currentZoomFactor = 1;
        window.electronAPI.setZoomFactor(currentZoomFactor);
        return;
    }

    if (event.ctrlKey) {
        event.preventDefault(); // Prevent the page from scrolling
        if      (event.deltaY < 0) { currentZoomFactor += 0.1; }
        else if (event.deltaY > 0) { currentZoomFactor = Math.max(0.1, currentZoomFactor - 0.1); }
        window.electronAPI.setZoomFactor(currentZoomFactor);
        return;
    }
});

// setting up keyboard events for topbar
document.addEventListener('keydown', (event) => {
    switch (event.key) {
        case 'p': hoverZoom_check(); break;
        case 'ArrowLeft': slider_left(); break;
        case 'ArrowRight': slider_right(); break;
    }
});


