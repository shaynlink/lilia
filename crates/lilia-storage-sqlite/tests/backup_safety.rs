use lilia_storage_sqlite::{Database, DatabaseOptions};
use std::fs;

#[test]
fn existing_destinations_and_source_aliases_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.lilia");
    let db = Database::open(DatabaseOptions::durable(&source)).unwrap();
    let existing = dir.path().join("existing.lilia");
    fs::write(&existing, b"do not overwrite").unwrap();
    assert!(db.backup(&existing).is_err());
    assert_eq!(fs::read(&existing).unwrap(), b"do not overwrite");
    assert!(db.backup(&source).is_err());
    assert!(db
        .backup(dir.path().join(".").join("source.lilia"))
        .is_err());
    let alias = dir.path().join("alias.lilia");
    fs::hard_link(&source, &alias).unwrap();
    assert!(db.backup(&alias).is_err());
    assert!(db.integrity_check().unwrap());
}

#[test]
fn concurrent_backups_publish_exactly_once_and_clean_staging() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(DatabaseOptions::durable(dir.path().join("source.lilia"))).unwrap();
    let destination = dir.path().join("snapshot.lilia");
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let run = || {
            barrier.wait();
            db.backup(&destination)
        };
        let first = scope.spawn(run);
        let second = scope.spawn(run);
        let results = [first.join().unwrap(), second.join().unwrap()];
        let successes = results.iter().filter(|result| result.is_ok()).count();
        assert_eq!(successes, 1, "backup results: {results:?}");
    });
    assert!(Database::open(DatabaseOptions::durable(destination))
        .unwrap()
        .integrity_check()
        .unwrap());
    assert!(fs::read_dir(dir.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".lilia-backup-")
    }));
}

#[cfg(unix)]
#[test]
fn symlink_destinations_are_not_followed() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.lilia");
    let db = Database::open(DatabaseOptions::durable(&source)).unwrap();
    for (name, target) in [("live", source), ("dangling", dir.path().join("missing"))] {
        let link = dir.path().join(name);
        symlink(&target, &link).unwrap();
        assert!(db.backup(&link).is_err());
        assert_eq!(fs::read_link(link).unwrap(), target);
    }
    assert!(!dir.path().join("missing").exists());
    assert!(db.integrity_check().unwrap());
}

#[cfg(unix)]
#[test]
fn parent_permissions_are_preserved_and_new_directories_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o750)).unwrap();
    let db = Database::open(DatabaseOptions::durable(dir.path().join("source.lilia"))).unwrap();
    let destination = dir.path().join("new/nested/snapshot.lilia");
    db.backup(&destination).unwrap();
    for (path, mode) in [
        (dir.path().to_path_buf(), 0o750),
        (dir.path().join("new"), 0o700),
        (dir.path().join("new/nested"), 0o700),
        (destination, 0o600),
    ] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            mode
        );
    }
}
