use comandos_store::unified::{self, Mode, Origin};
use std::{fs, os::unix::fs::DirBuilderExt};

#[test]
fn runtime_reads_survive_a_concurrent_payload_commit_without_changing_authority() {
    let home = std::env::temp_dir().join(format!("live-read-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&home).unwrap();
    let db = unified::open_unified(&unified::unified_path(&home)).unwrap();
    for mode in [Mode::Mirror, Mode::Unified, Mode::Sealed] {
        unified::set_mode(&db, "ui-docs", mode, "test", 1).unwrap();
        let result =
            comandos_store::domains::caller::read(&home, "ui-docs", |observed, _reader| {
                // Another SQLite connection commits while this runtime reader owns
                // its mode lease. A payload commit is not a change of authority.
                unified::doc_put(&db, "hooks/prefs.json", "ui-docs", b"{}", Origin::Mirror, 2)?;
                Ok(observed)
            });
        assert_eq!(result.unwrap(), mode);
        assert_eq!(unified::mode_of(Some(&db), "ui-docs").unwrap(), mode);
    }
    drop(db);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn runtime_reader_yields_when_a_mode_change_owns_the_lease() {
    let home = std::env::temp_dir().join(format!("live-mode-busy-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&home).unwrap();
    let path = unified::unified_path(&home);
    let db = unified::open_unified(&path).unwrap();
    let lock = comandos_store::files::FileLock::exclusive(
        &path.with_file_name("comandos.sqlite3.domain-modes.lock"),
    )
    .unwrap();
    let result = comandos_store::domains::caller::read(&home, "ui-docs", |_, _| Ok(()));
    assert!(matches!(result, Err(comandos_store::Error::ModeBusy)));
    drop(lock);
    drop(db);
    fs::remove_dir_all(home).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn repeated_runtime_reads_do_not_copy_unrelated_wal_payloads() {
    fn read_bytes() -> u64 {
        fs::read_to_string("/proc/thread-self/io")
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("rchar: "))
            .unwrap()
            .parse()
            .unwrap()
    }
    let home = std::env::temp_dir().join(format!("live-read-io-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&home).unwrap();
    let db = unified::open_unified(&unified::unified_path(&home)).unwrap();
    db.execute_batch("PRAGMA wal_autocheckpoint=0; CREATE TABLE unrelated_payload(body BLOB); INSERT INTO unrelated_payload VALUES(zeroblob(4194304));").unwrap();
    unified::set_mode(&db, "ui-docs", Mode::Unified, "test", 1).unwrap();
    let read = || {
        comandos_store::domains::caller::read(&home, "ui-docs", |mode, reader| {
            assert_eq!(mode, Mode::Unified);
            unified::doc_get(reader.unwrap(), "hooks/prefs.json")
        })
        .unwrap()
    };
    assert!(read().is_none());
    let before = read_bytes();
    for value in 0..6 {
        let body = value.to_string();
        unified::doc_put(
            &db,
            "hooks/prefs.json",
            "ui-docs",
            body.as_bytes(),
            Origin::Unified,
            value,
        )
        .unwrap();
        assert_eq!(read().unwrap().body, body.as_bytes());
    }
    let bytes = read_bytes() - before;
    eprintln!("six_runtime_reads_bytes={bytes}");
    let lock = comandos_store::files::FileLock::exclusive(
        &unified::unified_path(&home).with_file_name("comandos.sqlite3.domain-modes.lock"),
    )
    .unwrap();
    let before_busy = read_bytes();
    for _ in 0..6 {
        assert!(matches!(
            comandos_store::domains::caller::CallerAccess::open_read(&home, "ui-docs"),
            Err(comandos_store::Error::ModeBusy)
        ));
    }
    let busy_bytes = read_bytes() - before_busy;
    drop(lock);
    drop(db);
    fs::remove_dir_all(home).unwrap();
    assert!(
        bytes < 512 * 1024,
        "six live reads consumed {bytes} bytes for six tiny documents"
    );
    assert!(
        busy_bytes < 512 * 1024,
        "six busy admissions consumed {busy_bytes} bytes"
    );
}

#[test]
fn runtime_read_cache_observes_another_process_commit_and_mode_change() {
    let home = std::env::temp_dir().join(format!("live-read-process-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&home).unwrap();
    let db = unified::open_unified(&unified::unified_path(&home)).unwrap();
    let read = || {
        comandos_store::domains::caller::read(&home, "ui-docs", |mode, reader| {
            Ok((mode, unified::doc_get(reader.unwrap(), "hooks/prefs.json")?))
        })
        .unwrap()
    };
    assert_eq!(read().0, Mode::Legacy);
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "runtime_read_external_writer", "--ignored"])
        .env("COMANDOS_TEST_LIVE_READER_HOME", &home)
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    let (mode, doc) = read();
    assert_eq!(mode, Mode::Sealed);
    assert_eq!(doc.unwrap().body, b"committed by child");
    drop(db);
    fs::remove_dir_all(home).unwrap();
}

#[test]
#[ignore = "private child process used by runtime_read_cache_observes_another_process_commit_and_mode_change"]
fn runtime_read_external_writer() {
    let Some(home) = std::env::var_os("COMANDOS_TEST_LIVE_READER_HOME") else {
        return;
    };
    let home = std::path::PathBuf::from(home);
    let db = unified::open_unified(&unified::unified_path(&home)).unwrap();
    unified::doc_put(
        &db,
        "hooks/prefs.json",
        "ui-docs",
        b"committed by child",
        Origin::Unified,
        2,
    )
    .unwrap();
    unified::set_mode(&db, "ui-docs", Mode::Sealed, "child process", 2).unwrap();
}

#[test]
fn cached_runtime_reader_rejects_newer_and_incomplete_schemas() {
    let home = std::env::temp_dir().join(format!("live-read-schema-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&home).unwrap();
    let path = unified::unified_path(&home);
    let db = unified::open_unified(&path).unwrap();
    let read = || comandos_store::domains::caller::read(&home, "ui-docs", |_, _| Ok(()));
    read().unwrap();
    db.pragma_update(None, "user_version", 99).unwrap();
    let wal = path.with_file_name("comandos.sqlite3-wal");
    let before = (fs::read(&path).unwrap(), fs::read(&wal).unwrap());
    assert!(read().is_err());
    assert_eq!(before, (fs::read(&path).unwrap(), fs::read(&wal).unwrap()));
    db.pragma_update(None, "user_version", comandos_store::usage::SCHEMA_VERSION)
        .unwrap();
    read().unwrap();
    db.execute("DELETE FROM schema_migrations WHERE version=101", [])
        .unwrap();
    assert!(read().is_err());
    drop(db);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn cached_runtime_reader_reopens_a_replaced_database() {
    let home = std::env::temp_dir().join(format!("live-read-replaced-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&home).unwrap();
    let path = unified::unified_path(&home);
    let old = unified::open_unified(&path).unwrap();
    let read =
        || comandos_store::domains::caller::read(&home, "ui-docs", |mode, _| Ok(mode)).unwrap();
    assert_eq!(read(), Mode::Legacy);
    let parent = path.parent().unwrap();
    let moved = parent.with_file_name("previous-comandos");
    fs::rename(parent, &moved).unwrap();
    let new = unified::open_unified(&path).unwrap();
    unified::set_mode(&new, "ui-docs", Mode::Unified, "replacement", 2).unwrap();
    assert_eq!(read(), Mode::Unified);
    drop(old);
    drop(new);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn background_reads_wait_out_a_brief_mode_change() {
    let home = std::env::temp_dir().join(format!("live-wait-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&home).unwrap();
    let path = unified::unified_path(&home);
    let db = unified::open_unified(&path).unwrap();
    let lock = comandos_store::files::FileLock::exclusive(
        &path.with_file_name("comandos.sqlite3.domain-modes.lock"),
    )
    .unwrap();
    let read = || comandos_store::domains::caller::read(&home, "ui-docs", |_, _| Ok(()));
    // Request threads still fail at once...
    assert!(matches!(read(), Err(comandos_store::Error::ModeBusy)));
    // ...while a worker waits for the writer to finish.
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(60));
        drop(lock);
    });
    let waited = comandos_store::wait_admitted(std::time::Duration::from_secs(2), read);
    assert!(waited.is_ok(), "{waited:?}");
    release.join().unwrap();
    // A writer that never leaves still ends in ModeBusy once the wait runs out.
    let held = comandos_store::files::FileLock::exclusive(
        &path.with_file_name("comandos.sqlite3.domain-modes.lock"),
    )
    .unwrap();
    let start = std::time::Instant::now();
    let busy = comandos_store::wait_admitted(std::time::Duration::from_millis(50), read);
    assert!(matches!(busy, Err(comandos_store::Error::ModeBusy)));
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
    drop(held);
    drop(db);
    fs::remove_dir_all(home).unwrap();
}
