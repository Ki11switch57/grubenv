use grubenv::{Error, GrubEnv};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("grubenv-test-{}-{}", std::process::id(), id));
        fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn create_writes_padded_empty_block() {
    let dir = TempDir::new();
    let path = dir.file("grubenv");
    let env = GrubEnv::create(&path).unwrap();
    assert_eq!(env.list().count(), 0);
    let bytes = fs::read(&path).unwrap();
    assert_eq!(bytes.len(), 1024);
    assert!(bytes.starts_with(b"# GRUB Environment Block\n"));
    assert!(bytes[25..].iter().all(|&b| b == b'#'));
}

#[test]
fn set_saves_immediately() {
    let dir = TempDir::new();
    let path = dir.file("grubenv");
    let mut env = GrubEnv::create(&path).unwrap();
    env.set("saved_entry", "Fedora").unwrap();
    assert_eq!(env.get("saved_entry"), Some("Fedora"));
    let reloaded = GrubEnv::load(&path).unwrap();
    assert_eq!(reloaded.get("saved_entry"), Some("Fedora"));
    assert_eq!(fs::metadata(&path).unwrap().len(), 1024);
}

#[test]
fn set_overwrites_and_unset_removes_on_disk() {
    let dir = TempDir::new();
    let path = dir.file("grubenv");
    let mut env = GrubEnv::create(&path).unwrap();
    env.set("a", "1").unwrap();
    env.set("a", "2").unwrap();
    env.set("b", "x").unwrap();
    assert_eq!(GrubEnv::load(&path).unwrap().get("a"), Some("2"));
    env.unset("a").unwrap();
    env.unset("missing").unwrap();
    let reloaded = GrubEnv::load(&path).unwrap();
    assert_eq!(reloaded.get("a"), None);
    assert_eq!(reloaded.list().collect::<Vec<_>>(), [("b", "x")]);
}

#[test]
fn escapes_roundtrip() {
    let dir = TempDir::new();
    let path = dir.file("grubenv");
    let mut env = GrubEnv::create(&path).unwrap();
    env.set("k", "a\\b\nc").unwrap();
    let raw = fs::read_to_string(&path).unwrap();
    assert!(raw.contains("k=a\\\\b\\\nc\n"));
    assert_eq!(GrubEnv::load(&path).unwrap().get("k"), Some("a\\b\nc"));
}

#[test]
fn empty_value_is_kept() {
    let dir = TempDir::new();
    let path = dir.file("grubenv");
    let mut env = GrubEnv::create(&path).unwrap();
    env.set("e", "").unwrap();
    assert_eq!(GrubEnv::load(&path).unwrap().get("e"), Some(""));
}

#[test]
fn loads_real_world_sample() {
    let dir = TempDir::new();
    let path = dir.file("grubenv");
    let mut data =
        b"# GRUB Environment Block\nsaved_entry=Advanced options>Linux 6.1\nboot_success=1\n"
            .to_vec();
    data.resize(1024, b'#');
    fs::write(&path, &data).unwrap();
    let env = GrubEnv::load(&path).unwrap();
    assert_eq!(env.get("saved_entry"), Some("Advanced options>Linux 6.1"));
    assert_eq!(env.get("boot_success"), Some("1"));
}

#[test]
fn zero_padding_and_duplicates() {
    let dir = TempDir::new();
    let path = dir.file("grubenv");
    let mut data = b"# GRUB Environment Block\na=1\na=2\n".to_vec();
    data.resize(64, 0);
    fs::write(&path, &data).unwrap();
    let env = GrubEnv::load(&path).unwrap();
    assert_eq!(env.list().count(), 1);
    assert_eq!(env.get("a"), Some("2"));
}

#[test]
fn load_errors() {
    let dir = TempDir::new();
    assert!(matches!(
        GrubEnv::load(dir.file("missing")),
        Err(Error::Io(_))
    ));

    let bad = dir.file("bad");
    fs::write(&bad, b"not a grubenv").unwrap();
    assert!(matches!(GrubEnv::load(&bad), Err(Error::InvalidSignature)));

    let malformed = dir.file("malformed");
    let mut data = b"# GRUB Environment Block\nnoequals\n".to_vec();
    data.resize(64, b'#');
    fs::write(&malformed, &data).unwrap();
    assert!(matches!(
        GrubEnv::load(&malformed),
        Err(Error::Malformed { offset: 25 })
    ));
}

#[test]
fn rejects_invalid_names_and_values() {
    let dir = TempDir::new();
    let path = dir.file("grubenv");
    let mut env = GrubEnv::create(&path).unwrap();
    for name in ["", "a=b", "a\nb", "#x", "a\0b"] {
        assert!(matches!(env.set(name, "v"), Err(Error::InvalidName(_))));
    }
    assert!(matches!(env.set("k", "a\0b"), Err(Error::InvalidValue(_))));
    assert_eq!(env.list().count(), 0);
}

#[test]
fn full_block_fails_without_changing_state() {
    let dir = TempDir::new();
    let path = dir.file("grubenv");
    let mut env = GrubEnv::create(&path).unwrap();
    env.set("big", &"x".repeat(900)).unwrap();
    let before = fs::read(&path).unwrap();
    let err = env.set("more", &"y".repeat(200)).unwrap_err();
    assert!(matches!(err, Error::NoSpace { .. }));
    assert_eq!(env.get("more"), None);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn leaves_no_temp_files() {
    let dir = TempDir::new();
    let path = dir.file("grubenv");
    let mut env = GrubEnv::create(&path).unwrap();
    env.set("k", "v").unwrap();
    env.unset("k").unwrap();
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn save_follows_symlink() {
    let dir = TempDir::new();
    let real = dir.file("real");
    let link = dir.file("link");
    GrubEnv::create(&real).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let mut env = GrubEnv::load(&link).unwrap();
    env.set("k", "v").unwrap();

    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(GrubEnv::load(&real).unwrap().get("k"), Some("v"));
}
