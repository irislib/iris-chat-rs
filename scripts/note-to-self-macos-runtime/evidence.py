"""Source-bound product and Note to Self witness checks; no live UI operations."""
from contextlib import closing
import hashlib
import json
import re
import shutil
import sqlite3
import tarfile
import zipfile
from pathlib import Path

REPO = "irislib/iris-chat-rs"
APP = Path("Products/Debug/Iris Chat.app")
CLI = Path("Products/CLI/iris")
PREFIX = "iris-direct-file-v1:"


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha256(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as source:
        for chunk in iter(lambda: source.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def product_identity(source, run_id, artifact_id):
    require(isinstance(source, str) and re.fullmatch(r"[a-f0-9]{40}", source)
            and type(run_id) is int and run_id > 0 and type(artifact_id) is int and artifact_id > 0,
            "Explicit full product source, run ID and artifact ID are required")


def product_spec(run, artifact, tree, *, source, run_id, artifact_id):
    product_identity(source, run_id, artifact_id)
    require(run["id"] == run_id and run["head_sha"] == source
            and run["repository"]["full_name"] == REPO
            and run["path"] in (".github/workflows/ci.yml", ".github/workflows/apple-validation-products.yml")
            and run["status"] == "completed" and run["conclusion"] == "success",
            "Expected successful immutable candidate CI run")
    require(artifact["id"] == artifact_id and artifact["name"] == f"macos-validation-{source}"
            and not artifact["expired"] and artifact["workflow_run"]["id"] == run_id
            and artifact["workflow_run"]["head_sha"] == source
            and re.fullmatch(r"sha256:[a-f0-9]{64}", artifact["digest"]) and re.fullmatch(r"[a-f0-9]{40}", tree),
            "Artifact identity or checksum differs from the candidate")
    return {"id": artifact["id"], "source": source, "run": run_id, "workflow": run["path"],
            "bytes": artifact["size_in_bytes"],
            "sha256": artifact["digest"].removeprefix("sha256:"), "tree": tree}


def unpack_product(archive, root, spec, reserve):
    require(archive.is_file() and archive.stat().st_size == spec["bytes"]
            and sha256(archive) == spec["sha256"], "Outer artifact checksum mismatch")
    packed = root / "macos-products.tar.gz"
    with zipfile.ZipFile(archive) as bundle:
        for name in ("build-receipt.json", packed.name):
            require(bundle.namelist().count(name) == 1, "Missing or ambiguous product member")
        receipt = json.loads(bundle.read("build-receipt.json"))
        require(receipt["commit"] == spec["source"] and receipt["tree"] == spec["tree"]
                and receipt["architecture"] == "arm64" and receipt["platform"] == "macos"
                and receipt["configuration"] == "Debug" and receipt["rustProfile"] == "debug"
                and receipt["signed"] is False and receipt["testsExecuted"] is False
                and receipt["releasePackage"] is False
                and receipt["archive"] == packed.name, "Product receipt does not match the candidate")
        size = bundle.getinfo(packed.name).file_size
        require(size == receipt["archiveBytes"] and size < shutil.disk_usage(root).free - reserve,
                "Product archive size exceeds expected size or available space")
        with bundle.open(packed.name) as src, packed.open("xb") as dst:
            shutil.copyfileobj(src, dst)
    require(sha256(packed) == receipt["archiveSHA256"], "Inner product checksum mismatch")
    products = root / "products"
    products.mkdir()
    with tarfile.open(packed, "r:gz") as tar:
        members = [m for m in tar if Path(m.name) in (CLI, APP) or APP in Path(m.name).parents]
        require(sum(m.size for m in members) < shutil.disk_usage(root).free - reserve,
                "Insufficient free space for the app and CLI")
        require(len({m.name for m in members}) == len(members)
                and all(m.isfile() or m.isdir() or m.issym() for m in members),
                "Ambiguous or unsupported archive entries")
        tar.extractall(products, members=members, filter="data")
    return products


def self_offer(profile, owner, source, filename):
    database = profile / "core.sqlite3"
    if not database.exists():
        return None
    # The UI can decorate away the signed offer text. Check the raw durable
    # self-chat row as well as its separate visible transfer card.
    with closing(sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True)) as db, db:
        rows = db.execute("SELECT id,body,author_owner_pubkey_hex,is_outgoing FROM messages "
                          "WHERE chat_id=? AND body LIKE 'iris-direct-file-v1:%'", (owner,)).fetchall()
    matches = []
    for message_id, body, author, outgoing in rows:
        wire = json.loads(body[len(PREFIX):])
        offer = json.loads(wire["content"])
        files = offer["files"]
        if not any(f["filename"] == filename for f in files):
            continue
        require(author == owner == offer["owner"] == offer["recipient"] and outgoing
                and wire["pubkey"] == source == offer["device"], "Wrong account or source device")
        canonical = json.dumps([0, wire["pubkey"], wire["created_at"], wire["kind"], wire["tags"], wire["content"]],
                               separators=(",", ":"), ensure_ascii=False).encode()
        require(wire["kind"] == 21111 and hashlib.sha256(canonical).hexdigest() == wire["id"],
                "Invalid signed offer content ID")
        require(len(files) == 1 and re.fullmatch("[a-f0-9]{32}", offer["id"]),
                "Invalid single-file offer")
        matches.append({"message": message_id, "transfer": offer["id"], "wire": wire["id"],
                        "size": files[0]["size_bytes"], "sha256": files[0]["sha256"]})
    require(len(matches) < 2, "Duplicate Note to Self file offer")
    return matches[0] if matches else None


def compare_copies(original, copies, payload):
    require(original is not None and all(copy == original for copy in copies), "Offer IDs or contents differ")
    require(original["size"] == payload["size"] and original["sha256"] == payload["sha256"],
            "Offer manifest does not identify the selected file")


def received_file(folder, offer, filename):
    target = folder / ("Iris files " + offer["transfer"]) / ("1-" + filename)
    files = [p for p in folder.rglob("*") if p.is_file() or p.is_symlink()]
    require(files == [target] and not target.is_symlink() and not target.parent.is_symlink(),
            "Unexpected or missing destination file")
    require(target.stat().st_size == offer["size"] and sha256(target) == offer["sha256"],
            "Accepted bytes do not match the selected file")
    return sha256(target)


def sibling_retry_state(profile, owner, source, ready, late, message):
    db_path = (profile / "core.sqlite3").resolve()
    with closing(sqlite3.connect(db_path.as_uri() + "?mode=ro", uri=True)) as db, db:
        rows = db.execute(
            "SELECT data FROM ndr_kv_checkpoint_chunks WHERE owner_pubkey_hex=? AND "
            "device_pubkey_hex=? AND key=? AND payload_key=? ORDER BY chunk_index",
            (owner, source, "appcore/protocol-engine-state-v1", b"\0core"),
        ).fetchall()
    if not rows:
        return "unavailable"
    # The 'core' part closes the pending_inbound array, then contains the
    # ordinary protocol state. Keep that private JSON entirely on the sender.
    state = json.loads(b'{"pending_inbound":[' + b"".join(r[0] for r in rows))
    pending = [p for p in state["pending_local_sibling_sends"]
               if p["chat_id"] == owner and p["message_id"] == message]
    if not pending:
        return "drained"
    if (len(pending) == 1 and set(pending[0]["eligible_devices"]) == {ready, late}
            and set(pending[0]["completed_devices"]) == {ready}):
        return "receiver-complete"
    return "other"
