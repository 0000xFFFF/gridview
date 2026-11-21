//! Is the volume a file lives on encrypted? (Linux)
//!
//! Everything comes from what the kernel exposes, no root needed:
//! * `/proc/self/mountinfo` says which device / filesystem a path is on,
//! * `/sys/dev/block/<major>:<minor>/dm/uuid` starts with `CRYPT-` for
//!   device-mapper crypt targets (LUKS, plain dm-crypt, VeraCrypt, BitLocker
//!   through cryptsetup). Devices stacked on top of one (LVM, md, ...) are found
//!   by following `slaves/`,
//! * encrypting stacked filesystems (eCryptfs, gocryptfs, encfs, cryfs, ...)
//!   are recognised by their filesystem type.
//!
//! Not detected: fscrypt (per-directory encryption of ext4/f2fs) and native ZFS
//! encryption.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub major: u32,
    pub minor: u32,
    pub mount_point: PathBuf,
    pub fstype: String,
    pub source: String,
}

/// mountinfo escapes space, tab, newline and backslash as `\040` style octal.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let octal = b
            .get(i + 1..i + 4)
            .filter(|d| d.iter().all(|c| (b'0'..=b'7').contains(c)));
        match (b[i], octal) {
            (b'\\', Some(d)) => {
                let v = (d[0] - b'0') as u32 * 64 + (d[1] - b'0') as u32 * 8 + (d[2] - b'0') as u32;
                out.push(v as u8);
                i += 4;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 - ext3 /dev/root rw,errors=continue`
pub fn parse_mountinfo(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            // fields 0..=5 are fixed, then optional fields, then "-"
            let dash = f.iter().position(|&x| x == "-")?;
            if dash < 6 || f.len() < dash + 3 {
                return None;
            }
            let (major, minor) = parse_majmin(f[2])?;
            Some(Mount {
                major,
                minor,
                mount_point: PathBuf::from(unescape(f[4])),
                fstype: f[dash + 1].to_string(),
                source: unescape(f[dash + 2]),
            })
        })
        .collect()
}

fn parse_majmin(s: &str) -> Option<(u32, u32)> {
    let (a, b) = s.trim().split_once(':')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// Major / minor of a `st_dev` / `st_rdev` value (glibc encoding).
pub fn split_dev(dev: u64) -> (u32, u32) {
    let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff);
    let minor = (dev & 0xff) | ((dev >> 12) & !0xff);
    (major as u32, minor as u32)
}

const CRYPT_FS: &[&str] = &[
    "ecryptfs",
    "gocryptfs",
    "encfs",
    "cryfs",
    "securefs",
    "cryptomator",
    "veracrypt",
    "truecrypt",
];

/// Filesystems whose whole purpose is to encrypt (`fuse.gocryptfs`, `ecryptfs`, ...).
pub fn is_crypt_fs(fstype: &str) -> bool {
    let t = fstype.to_ascii_lowercase();
    CRYPT_FS.iter().any(|n| t.contains(n))
}

fn read_trim(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

/// Is block device `major:minor` a device-mapper crypt target, or stacked on one?
/// `sys` is the sysfs root (`/sys`).
pub fn dm_encrypted(sys: &Path, major: u32, minor: u32, depth: u32) -> bool {
    if depth > 8 {
        return false; // loops / absurdly deep stacks
    }
    let dev = sys.join("dev/block").join(format!("{major}:{minor}"));
    if read_trim(&dev.join("dm/uuid")).is_some_and(|u| u.starts_with("CRYPT-")) {
        return true;
    }
    if read_trim(&dev.join("dm/name")).is_some_and(|n| {
        let n = n.to_ascii_lowercase();
        n.starts_with("veracrypt") || n.starts_with("truecrypt")
    }) {
        return true;
    }
    // LVM, md, another dm layer on top of an encrypted device
    let Ok(slaves) = fs::read_dir(dev.join("slaves")) else {
        return false;
    };
    slaves.flatten().any(|e| {
        read_trim(&e.path().join("dev"))
            .and_then(|s| parse_majmin(&s))
            .is_some_and(|(a, b)| dm_encrypted(sys, a, b, depth + 1))
    })
}

/// Device numbers behind a mount source such as `/dev/mapper/cryptroot`. Needed
/// for filesystems with an anonymous device number (btrfs, ...).
fn source_dev(source: &str) -> Option<(u32, u32)> {
    if !source.starts_with("/dev/") {
        return None;
    }
    let md = fs::metadata(fs::canonicalize(source).ok()?).ok()?;
    Some(split_dev(md.rdev()))
}

pub struct Detector {
    mounts: Vec<Mount>,
    sys: PathBuf,
    resolve_source: fn(&str) -> Option<(u32, u32)>,
}

impl Detector {
    /// Reads the live mount table.
    pub fn system() -> Self {
        let text = fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
        Self::new(parse_mountinfo(&text), PathBuf::from("/sys"), source_dev)
    }

    pub fn new(
        mounts: Vec<Mount>,
        sys: PathBuf,
        resolve_source: fn(&str) -> Option<(u32, u32)>,
    ) -> Self {
        Self {
            mounts,
            sys,
            resolve_source,
        }
    }

    /// The mount that `path` (canonical) is on. `dev` is its `st_dev`. If a
    /// device is mounted more than once (bind mounts) the one containing `path`
    /// with the longest mount point wins.
    pub fn mount_for(&self, dev: u64, path: &Path) -> Option<&Mount> {
        let (major, minor) = split_dev(dev);
        self.mounts
            .iter()
            .filter(|m| m.major == major && m.minor == minor)
            .max_by_key(|m| {
                if path.starts_with(&m.mount_point) {
                    m.mount_point.components().count()
                } else {
                    0
                }
            })
    }

    pub fn is_encrypted(&self, m: &Mount) -> bool {
        if is_crypt_fs(&m.fstype) {
            return true;
        }
        let (major, minor) = if m.major != 0 {
            (m.major, m.minor)
        } else {
            match (self.resolve_source)(&m.source) {
                Some(d) => d,
                None => return false,
            }
        };
        dm_encrypted(&self.sys, major, minor, 0)
    }

    /// Mount point of the encrypted volume `path` lives on, `None` if it is not encrypted.
    pub fn encrypted_mount(&self, dev: u64, path: &Path) -> Option<PathBuf> {
        let m = self.mount_for(dev, path)?;
        self.is_encrypted(m).then(|| m.mount_point.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn makedev(major: u64, minor: u64) -> u64 {
        ((major & 0xfff) << 8) | (minor & 0xff) | ((minor & !0xff) << 12) | ((major & !0xfff) << 32)
    }

    /// A throw-away sysfs tree.
    struct Sys(PathBuf);
    impl Sys {
        fn new() -> Self {
            static N: AtomicU32 = AtomicU32::new(0);
            let p = std::env::temp_dir().join(format!(
                "gv-sys-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(p.join("dev/block")).unwrap();
            Sys(p)
        }
        fn dev(&self, maj: u32, min: u32) -> PathBuf {
            let d = self.0.join("dev/block").join(format!("{maj}:{min}"));
            fs::create_dir_all(&d).unwrap();
            d
        }
        fn dm(&self, maj: u32, min: u32, uuid: &str, name: &str) {
            let d = self.dev(maj, min).join("dm");
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("uuid"), format!("{uuid}\n")).unwrap();
            fs::write(d.join("name"), format!("{name}\n")).unwrap();
        }
        /// `top` sits on top of `below`.
        fn stack(&self, top: (u32, u32), below: (u32, u32), name: &str) {
            let s = self.dev(top.0, top.1).join("slaves").join(name);
            fs::create_dir_all(&s).unwrap();
            fs::write(s.join("dev"), format!("{}:{}\n", below.0, below.1)).unwrap();
        }
    }
    impl Drop for Sys {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn split_dev_roundtrips_real_numbers() {
        for (a, b) in [(8, 1), (254, 3), (259, 2), (0, 45), (0, 300), (4095, 1000)] {
            assert_eq!(split_dev(makedev(a, b)), (a as u32, b as u32));
        }
    }

    #[test]
    fn mountinfo_parsing() {
        let text = "\
36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 - ext3 /dev/root rw,errors=continue
40 36 254:7 / /media/me/My\\040Vault rw - ext4 /dev/mapper/vault rw
41 36 0:45 / /mnt/x rw shared:3 master:4 - btrfs /dev/mapper/crypt2 rw
garbage line
";
        let m = parse_mountinfo(text);
        assert_eq!(m.len(), 3);
        assert_eq!((m[0].major, m[0].minor), (98, 0));
        assert_eq!(m[0].mount_point, PathBuf::from("/mnt2"));
        assert_eq!(m[0].fstype, "ext3");
        assert_eq!(m[1].mount_point, PathBuf::from("/media/me/My Vault"));
        assert_eq!(m[2].source, "/dev/mapper/crypt2"); // optional fields skipped
    }

    #[test]
    fn encrypting_filesystems_are_recognised_by_type() {
        for t in [
            "ecryptfs",
            "fuse.gocryptfs",
            "fuse.encfs",
            "fuse.cryfs",
            "fuse.Cryptomator",
        ] {
            assert!(is_crypt_fs(t), "{t}");
        }
        for t in ["ext4", "btrfs", "ntfs3", "fuse.sshfs", "tmpfs", "overlay"] {
            assert!(!is_crypt_fs(t), "{t}");
        }
    }

    #[test]
    fn dm_crypt_targets_are_detected() {
        let s = Sys::new();
        s.dm(254, 0, "CRYPT-LUKS2-0123-luks-0123", "luks-0123");
        s.dm(254, 1, "CRYPT-PLAIN-secret", "secret");
        s.dm(254, 2, "", "veracrypt1");
        s.dm(254, 3, "LVM-abcdef", "vg-data");
        for min in 0..3 {
            assert!(dm_encrypted(&s.0, 254, min, 0), "254:{min}");
        }
        assert!(!dm_encrypted(&s.0, 254, 3, 0), "plain LVM");
        assert!(!dm_encrypted(&s.0, 8, 1, 0), "a partition without dm");
    }

    #[test]
    fn stacked_devices_inherit_encryption_from_below() {
        let s = Sys::new();
        s.dm(254, 0, "CRYPT-LUKS2-aaaa-pv", "pv"); // LUKS ...
        s.dm(254, 1, "LVM-root", "vg-root"); // ... with LVM on top
        s.stack((254, 1), (254, 0), "dm-0");
        s.dm(254, 2, "LVM-home", "vg2-home"); // LVM on a plain partition
        s.stack((254, 2), (8, 2), "sda2");
        s.dev(8, 2);
        assert!(dm_encrypted(&s.0, 254, 1, 0));
        assert!(!dm_encrypted(&s.0, 254, 2, 0));
    }

    #[test]
    fn device_loops_terminate() {
        let s = Sys::new();
        s.dm(254, 5, "LVM-x", "x");
        s.stack((254, 5), (254, 5), "dm-5");
        assert!(!dm_encrypted(&s.0, 254, 5, 0));
    }

    fn detector(s: &Sys) -> Detector {
        let text = "\
36 1 254:3 / / rw,relatime shared:1 - ext4 /dev/mapper/luks-root rw
37 36 8:1 / /boot rw,relatime shared:2 - ext4 /dev/sda1 rw
38 36 8:17 / /media/me/usb rw - exfat /dev/sdb1 rw
40 36 254:7 / /media/me/My\\040Vault rw - ext4 /dev/mapper/vault rw
41 36 0:45 / /mnt/btrfs rw - btrfs /dev/mapper/crypt2 rw
42 36 0:46 / /mnt/plainbtrfs rw - btrfs /dev/sdc1 rw
43 36 0:50 / /home/me/Private rw - ecryptfs /home/me/.Private rw
44 36 0:51 / /mnt/gc rw - fuse.gocryptfs gocryptfs rw
45 36 254:7 /photos /srv/photos-bind rw - ext4 /dev/mapper/vault rw
";
        fn resolve(src: &str) -> Option<(u32, u32)> {
            (src == "/dev/mapper/crypt2").then_some((254, 9))
        }
        s.dm(254, 3, "CRYPT-LUKS2-root-luks-root", "luks-root");
        s.dm(254, 7, "CRYPT-LUKS2-vault-vault", "vault");
        s.dm(254, 9, "CRYPT-LUKS2-c2-crypt2", "crypt2");
        Detector::new(parse_mountinfo(text), s.0.clone(), resolve)
    }

    #[test]
    fn detects_the_encrypted_volume_a_path_is_on() {
        let s = Sys::new();
        let d = detector(&s);
        let enc = |maj, min, p: &str| d.encrypted_mount(makedev(maj, min), Path::new(p));

        // whole system on LUKS
        assert_eq!(enc(254, 3, "/home/me/pics"), Some(PathBuf::from("/")));
        // unencrypted drives
        assert_eq!(enc(8, 1, "/boot/x"), None);
        assert_eq!(enc(8, 17, "/media/me/usb/DCIM"), None);
        // an unlocked vault, mount point with a space in it
        assert_eq!(
            enc(254, 7, "/media/me/My Vault/photos/a"),
            Some(PathBuf::from("/media/me/My Vault"))
        );
        // anonymous device number (btrfs): resolved through the mount source
        assert_eq!(
            enc(0, 45, "/mnt/btrfs/a"),
            Some(PathBuf::from("/mnt/btrfs"))
        );
        assert_eq!(enc(0, 46, "/mnt/plainbtrfs/a"), None);
        // stacked encrypting filesystems
        assert_eq!(
            enc(0, 50, "/home/me/Private/a"),
            Some(PathBuf::from("/home/me/Private"))
        );
        assert_eq!(enc(0, 51, "/mnt/gc/a"), Some(PathBuf::from("/mnt/gc")));
    }

    #[test]
    fn bind_mounts_pick_the_mount_the_path_is_under() {
        let s = Sys::new();
        let d = detector(&s);
        let m = d
            .mount_for(makedev(254, 7), Path::new("/srv/photos-bind/2024/a.jpg"))
            .unwrap();
        assert_eq!(m.mount_point, PathBuf::from("/srv/photos-bind"));
        let m = d
            .mount_for(makedev(254, 7), Path::new("/media/me/My Vault/x"))
            .unwrap();
        assert_eq!(m.mount_point, PathBuf::from("/media/me/My Vault"));
    }

    #[test]
    fn unknown_devices_are_not_encrypted() {
        let s = Sys::new();
        let d = detector(&s);
        assert_eq!(d.encrypted_mount(makedev(99, 99), Path::new("/x")), None);
    }

    #[test]
    fn real_system_does_not_panic() {
        // whatever this machine has mounted, detection must just answer
        let d = Detector::system();
        let md = fs::metadata("/").unwrap();
        let _ = d.encrypted_mount(md.dev(), Path::new("/"));
    }
}
