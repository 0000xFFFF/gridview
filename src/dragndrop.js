// Add event listeners for drag and drop functionality
const mediaDirs = document.getElementById('media-dirs');

mediaDirs.addEventListener('dragover', (event) => { event.preventDefault(); mediaDirs.classList.add('drag-over'); });
mediaDirs.addEventListener('dragleave', (event) => { event.preventDefault(); mediaDirs.classList.remove('drag-over'); });

mediaDirs.addEventListener('drop', (event) => {
    event.preventDefault();
    mediaDirs.classList.remove('drag-over');

    // Check if drop is from the web, and if it's an image, download it
    const items = event.dataTransfer.items;    
    for (const item of items) {
        if (item.kind === 'string' && item.type === 'text/uri-list') {
            item.getAsString((url) => { console.log(`${url}`); });
        }
    }

    // Check if drop is a local folder, and set our path to this folder if so
    const files = event.dataTransfer.files;
    for (let i = 0; i < files.length; i++) {
        const currentFile = files[i];
        if (!currentFile.type && currentFile.size % 4096 == 0) {
            ipcRenderer.send('drop-folder', currentFile.path);
            break;
        }
    }
});
