//! Bundled first-launch library; existing libraries are never reseeded.
use std::path::Path;
use std::fs;

include!(concat!(env!("OUT_DIR"), "/guides.rs"));

/// Any entry but the lock file means an existing library, even if all
/// documents were deleted; a pending directory is a first launch to finish.
fn seeded(root: &Path) -> std::io::Result<bool> {
    Ok(!root.join(".guides-pending").exists() && fs::read_dir(root)?.any(|e| e.map_or(true, |e| e.file_name() != ".lock")))
}

pub fn initialize(root: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(root)?;
    if seeded(root)? { return Ok(()); }
    // Copies of the app may share a library: two started together on a new
    // one seed it in turn. Released when `lock` drops.
    let lock = fs::OpenOptions::new().create(true).truncate(false).write(true).open(root.join(".lock"))?;
    lock.lock()?;
    if seeded(root)? { return Ok(()); }
    let pending = root.join(".guides-pending");
    fs::create_dir_all(&pending)?;
    // Stage complete files before publishing. The pending directory lets a
    // failed first launch resume without mistaking it for an existing library.
    for (name, bytes) in FILES {
        let path = pending.join(name);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, bytes)?;
    }
    for name in ["assets", "docs", "index.json", "format.json"] {
        let dest = root.join(name);
        if !dest.exists() { fs::rename(pending.join(name), dest)?; }
    }
    fs::remove_dir_all(pending)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Copies launched together on a new library: one seeds it, none fails.
    #[test]
    fn simultaneous_first_launches() {
        let root = std::env::temp_dir().join(format!("fdl-guides-{}", uuid::Uuid::new_v4()));
        let runs: Vec<_> = (0..4).map(|_| { let root = root.clone(); std::thread::spawn(move || super::initialize(&root)) }).collect();
        for r in runs { r.join().unwrap().unwrap(); }
        let docs = super::FILES.iter().filter(|(n, _)| n.starts_with("docs/")).count();
        assert!(docs > 0);
        assert_eq!(std::fs::read_dir(root.join("docs")).unwrap().count(), docs);
        assert!(!root.join(".guides-pending").exists());
        // A library with content is never reseeded.
        std::fs::remove_dir_all(root.join("docs")).unwrap();
        super::initialize(&root).unwrap();
        assert!(!root.join("docs").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
