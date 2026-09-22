//! Exercise publisher output with the same reader shipped in the app.
use std::path::Path;
use std::process::Command;

use hashtree_updater::{UpdateManifest, UpdateTarget};

fn staged_manifest(tag: &str) -> UpdateManifest {
    let directory = tempfile::tempdir().expect("release fixture directory");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/release-manifest.py");
    let output = Command::new("python3")
        .arg("-c")
        .arg(
            r#"
import importlib.util, pathlib, sys
spec = importlib.util.spec_from_file_location('release_manifest', sys.argv[1])
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)
root, tag = pathlib.Path(sys.argv[2]), sys.argv[3]
assets = root / 'assets'
assets.mkdir()
for name in publisher.asset_specs(tag):
    (assets / name).write_bytes(name.encode())
manifest = root / f'iris-chat-{tag}-manifest.json'
publisher.create_manifest(tag, 'test-commit', assets, manifest)
notes = root / 'notes.md'
notes.write_text('Fixture release.\n')
publisher.stage_hashtree(tag, manifest, assets, notes, root / 'stage', '2026-09-22T00:00:00Z')
print((root / 'stage' / 'release.json').read_text())
"#,
        )
        .arg(script)
        .arg(directory.path())
        .arg(tag)
        .output()
        .expect("stage release fixture");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("read staged manifest")
}

#[test]
fn published_manifest_is_accepted_by_shipped_updater() {
    for tag in ["v2026.9.22", "v2026.9.22.1"] {
        let manifest = staged_manifest(tag);
        manifest
            .validate()
            .expect("publisher and updater schemas agree");
        assert_eq!(manifest.tag.as_deref(), Some(tag));
        if tag.ends_with(".1") {
            assert_eq!(manifest.version, "2026.9.22+1");
        }
        assert!(manifest
            .select_asset(&UpdateTarget::new("aarch64-apple-darwin"))
            .is_some());
    }
}

#[test]
fn updater_can_compare_installed_corrective_release() {
    let manifest = UpdateManifest {
        version: "2026.9.22".to_string(),
        ..Default::default()
    };
    assert!(manifest
        .is_newer_than("2026.9.10.1")
        .expect("installed date version"));
}

#[tokio::test]
#[ignore = "requires the public release infrastructure"]
async fn public_signed_update_is_readable() {
    let (reference, updater) = iris_chat_core::update_announcements::build_secure_update_updater()
        .await
        .expect("prepare signed updater");
    let check = updater
        .check(hashtree_updater::UpdateCheckOptions {
            reference,
            current_version: "2026.9.10.1".to_string(),
            target: UpdateTarget::new("aarch64-apple-darwin"),
            ..Default::default()
        })
        .await
        .expect("resolve signed release");
    assert!(check.asset.is_some());
    println!("Resolved {:?} from signed discovery", check.manifest.tag);
}
