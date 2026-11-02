use super::*;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct TestChild(std::process::Child);
impl Drop for TestChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn only_one_owner_can_access_the_account() {
    let dir = tempfile::tempdir().unwrap();
    let first = CoreWriterLease::try_acquire(dir.path(), None, || Ok(()))
        .unwrap()
        .unwrap();
    let second = CoreWriterLease::try_acquire(dir.path(), None, || {
        panic!("a busy writer must not read or reload the account")
    })
    .unwrap();
    assert!(second.is_none());
    drop(first);
    assert!(CoreWriterLease::try_acquire(dir.path(), None, || Ok(()))
        .unwrap()
        .is_some());
}

#[test]
fn retained_core_reloads_only_after_another_owner() {
    let dir = tempfile::tempdir().unwrap();
    let first = CoreWriterLease::try_acquire(dir.path(), None, || Ok(()))
        .unwrap()
        .unwrap();
    let epoch = first.epoch();
    drop(first);
    let retained = CoreWriterLease::try_acquire(dir.path(), Some(epoch), || {
        panic!("no other owner has changed the account")
    })
    .unwrap()
    .unwrap();
    let epoch = retained.epoch();
    drop(retained);
    let extension = CoreWriterLease::try_acquire(dir.path(), None, || Ok(()))
        .unwrap()
        .unwrap();
    drop(extension);
    let mut reloaded = false;
    let resumed = CoreWriterLease::try_acquire(dir.path(), Some(epoch), || {
        reloaded = true;
        Ok(())
    })
    .unwrap()
    .unwrap();
    assert!(reloaded);
    assert!(resumed.epoch() != epoch);
}

#[test]
fn failed_reload_does_not_grant_writes_or_leave_a_valid_old_epoch() {
    let dir = tempfile::tempdir().unwrap();
    let first = CoreWriterLease::try_acquire(dir.path(), None, || Ok(()))
        .unwrap()
        .unwrap();
    let epoch = first.epoch();
    drop(first);
    let failed =
        CoreWriterLease::try_acquire(dir.path(), None, || anyhow::bail!("restore interrupted"));
    assert!(failed.is_err());
    let mut reloaded = false;
    let _resumed = CoreWriterLease::try_acquire(dir.path(), Some(epoch), || {
        reloaded = true;
        Ok(())
    })
    .unwrap()
    .unwrap();
    assert!(reloaded);
}

#[test]
fn incomplete_epoch_forces_reload_and_marker_contains_no_account_data() {
    let dir = tempfile::tempdir().unwrap();
    let first = CoreWriterLease::try_acquire(dir.path(), None, || Ok(()))
        .unwrap()
        .unwrap();
    let epoch = first.epoch();
    drop(first);
    std::fs::write(dir.path().join(LEASE_FILE), [0; 7]).unwrap();
    let mut reloaded = false;
    let _resumed = CoreWriterLease::try_acquire(dir.path(), Some(epoch), || {
        reloaded = true;
        Ok(())
    })
    .unwrap()
    .unwrap();
    assert!(reloaded);
    assert_eq!(
        std::fs::read(dir.path().join(LEASE_FILE)).unwrap().len(),
        EPOCH_BYTES
    );
}

// Re-enter only this test in a separate process. Its abrupt exit must release
// the OS lock without relying on Rust Drop or an application's cleanup handler.
#[test]
fn crashed_owner_requires_reload_before_the_next_write() {
    const CHILD_DIRECTORY: &str = "IRIS_TEST_WRITER_LEASE_DIRECTORY";
    if let Some(dir) = std::env::var_os(CHILD_DIRECTORY) {
        let dir = std::path::PathBuf::from(dir);
        let _lease = CoreWriterLease::try_acquire(&dir, None, || Ok(()))
            .unwrap()
            .unwrap();
        std::fs::write(dir.join("child-ready"), b"ready").unwrap();
        loop {
            std::thread::park();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let first = CoreWriterLease::try_acquire(dir.path(), None, || Ok(()))
        .unwrap()
        .unwrap();
    let epoch = first.epoch();
    drop(first);
    let mut child = TestChild(Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "core::storage::writer_lease::tests::crashed_owner_requires_reload_before_the_next_write", "--nocapture"])
        .env(CHILD_DIRECTORY, dir.path())
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::inherit())
        .spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !dir.path().join("child-ready").exists() && Instant::now() < deadline {
        if child.0.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let ready = dir.path().join("child-ready").exists();
    let blocked = if ready {
        CoreWriterLease::try_acquire(dir.path(), Some(epoch), || {
            panic!("live child owns the account")
        })
        .unwrap()
        .is_none()
    } else {
        false
    };
    let _ = child.0.kill();
    child.0.wait().unwrap();
    assert!(ready && blocked, "child ownership was not established");
    let mut reloaded = false;
    let _resumed = CoreWriterLease::try_acquire(dir.path(), Some(epoch), || {
        reloaded = true;
        Ok(())
    })
    .unwrap()
    .unwrap();
    assert!(reloaded);
}
