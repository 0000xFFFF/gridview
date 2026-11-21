//! Where cached data (thumbnails, image sizes) lives.
//!
//! Normally in the user's cache dir (`~/.cache/gridview`). But thumbnails are
//! pictures of the files, and the size cache lists their paths: if the files sit
//! on an encrypted drive, keeping those on the (unencrypted) system drive would
//! leak exactly what the encryption protects. So for files on an encrypted
//! volume the cache is kept on that volume, in `<mount point>/.gridview-cache`.
//!
//! The decision is made per file (a folder can contain mount points of other
//! drives) and memoised per directory. If an encrypted drive is not writable
//! nothing is cached for it, the thumbnails are simply regenerated each time.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;

#[cfg(target_os = "linux")]
use crate::encryption::Detector;

/// Name of the cache directory created on a drive.
pub const DRIVE_CACHE_DIR: &str = ".gridview-cache";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Policy {
    /// Cache on the drive itself for encrypted drives, in the user cache dir otherwise.
    Auto = 0,
    /// Always in the user cache dir.
    Central = 1,
    /// Always on the drive the files are on.
    Drive = 2,
}

static POLICY: AtomicU8 = AtomicU8::new(Policy::Auto as u8);

pub fn set_policy(p: Policy) {
    POLICY.store(p as u8, Ordering::Relaxed);
}

fn policy() -> Policy {
    match POLICY.load(Ordering::Relaxed) {
        1 => Policy::Central,
        2 => Policy::Drive,
        _ => Policy::Auto,
    }
}

/// The default cache dir (`~/.cache/gridview`).
pub fn central_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "gridview")
        .map(|d| d.cache_dir().to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir().join("gridview-cache"))
}

#[derive(Default)]
struct State {
    /// directory -> its cache dir (None: no persistent cache)
    by_parent: HashMap<PathBuf, Option<PathBuf>>,
    /// mount point -> prepared cache dir on it
    by_mount: HashMap<PathBuf, Option<PathBuf>>,
    /// volume the central cache dir is on
    central_vol: Option<Option<u64>>,
    #[cfg(target_os = "linux")]
    detector: Option<Detector>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

/// Forget everything that was looked up (mounts change: a vault may have been
/// unlocked since). Called whenever a new folder is opened.
pub fn reset() {
    *STATE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Cache directory for the file at `path`, `None` if there must be no
/// persistent cache for it.
pub fn dir_for(path: &Path) -> Option<PathBuf> {
    let Some(parent) = path.parent() else {
        return Some(central_dir());
    };
    if policy() == Policy::Central {
        return Some(central_dir());
    }
    with_state(|st| {
        if let Some(hit) = st.by_parent.get(parent) {
            return hit.clone();
        }
        let dir = match drive_mount(st, parent) {
            None => Some(central_dir()),
            Some(mount) => st
                .by_mount
                .entry(mount.clone())
                .or_insert_with(|| prepare_drive_dir(&mount))
                .clone(),
        };
        st.by_parent.insert(parent.to_path_buf(), dir.clone());
        dir
    })
}

/// Identifies the volume `p` is on (the device number).
#[cfg(unix)]
fn volume_id(p: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(p).ok().map(|m| m.dev())
}

/// Identifies the volume `p` is on (the drive letter / prefix).
#[cfg(not(unix))]
fn volume_id(p: &Path) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    volume_root(p).hash(&mut h);
    Some(h.finish())
}

/// `/` or `C:\`.
fn volume_root(p: &Path) -> PathBuf {
    p.ancestors()
        .last()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Closest ancestor of `p` that exists (the cache dir itself may not yet).
fn existing_ancestor(p: &Path) -> Option<&Path> {
    p.ancestors().find(|a| a.exists())
}

/// Mount point of the volume `parent` is on, if that volume needs a cache of
/// its own (it is encrypted, or the policy says so). `None`: use the central one.
fn drive_mount(st: &mut State, parent: &Path) -> Option<PathBuf> {
    let pol = policy();
    if pol == Policy::Central {
        return None;
    }
    let canon = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
    let vol = volume_id(&canon)?;

    // The central cache dir is on this very drive: it already is "inside the drive".
    let central = *st
        .central_vol
        .get_or_insert_with(|| existing_ancestor(&central_dir()).and_then(volume_id));
    if central == Some(vol) {
        return None;
    }

    #[cfg(target_os = "linux")]
    {
        let det = st.detector.get_or_insert_with(Detector::system);
        match pol {
            Policy::Auto => det.encrypted_mount(vol, &canon),
            Policy::Drive => det
                .mount_for(vol, &canon)
                .map(|m| m.mount_point.clone())
                .or_else(|| Some(volume_root(&canon))),
            Policy::Central => None,
        }
    }
    // No encryption detection on this platform: only when asked for.
    #[cfg(not(target_os = "linux"))]
    {
        (pol == Policy::Drive).then(|| volume_root(&canon))
    }
}

/// `<mount>/.gridview-cache`, created and checked for writability.
fn prepare_drive_dir(mount: &Path) -> Option<PathBuf> {
    let dir = mount.join(DRIVE_CACHE_DIR);
    let ok = fs::create_dir_all(&dir).is_ok() && {
        // an existing directory can still be read-only for us
        let probe = dir.join(".probe");
        let w = fs::write(&probe, b"").is_ok();
        let _ = fs::remove_file(&probe);
        w
    };
    if !ok {
        eprintln!(
            "gridview: cannot write a cache to {} - thumbnails of files there will not be cached",
            dir.display()
        );
        return None;
    }
    // keeps backup tools from archiving it
    let tag = dir.join("CACHEDIR.TAG");
    if !tag.exists() {
        let _ = fs::write(
            tag,
            "Signature: 8a477f597d28d172789f06886806bc55\n\
             # This file is a cache directory tag created by gridview.\n\
             # For information about cache directory tags see https://bford.info/cachedir/\n",
        );
    }
    Some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("gv-cache-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn drive_dir_is_created_tagged_and_idempotent() {
        let mount = tmp("mount");
        let d = prepare_drive_dir(&mount).unwrap();
        assert_eq!(d, mount.join(DRIVE_CACHE_DIR));
        let tag = fs::read_to_string(d.join("CACHEDIR.TAG")).unwrap();
        assert!(tag.starts_with("Signature: 8a477f597d28d172789f06886806bc55"));
        assert!(!d.join(".probe").exists(), "probe file is cleaned up");
        assert_eq!(prepare_drive_dir(&mount), Some(d));
        let _ = fs::remove_dir_all(&mount);
    }

    #[test]
    fn unwritable_drive_means_no_cache_not_a_fallback() {
        let base = tmp("ro");
        // a *file* where the directory has to go: creating it fails
        fs::write(base.join(DRIVE_CACHE_DIR), b"x").unwrap();
        assert_eq!(prepare_drive_dir(&base), None);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn volume_root_is_the_top_of_the_path() {
        #[cfg(unix)]
        assert_eq!(
            volume_root(Path::new("/media/me/vault/a.jpg")),
            PathBuf::from("/")
        );
        #[cfg(windows)]
        assert_eq!(
            volume_root(Path::new(r"D:\pics\a.jpg")),
            PathBuf::from(r"D:\")
        );
    }

    #[test]
    fn policy_roundtrip() {
        for p in [Policy::Central, Policy::Drive, Policy::Auto] {
            set_policy(p);
            assert_eq!(policy(), p);
        }
    }
}
