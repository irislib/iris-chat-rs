"""Offline acceptance-driver controls. These do not run Iris or control a GUI."""
from contextlib import closing
import copy
import hashlib
import io
import json
import re
import sqlite3
import tarfile
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path
from types import SimpleNamespace

from evidence import (APP, CLI, PREFIX, REPO, compare_copies, sibling_retry_state,
                      product_spec, received_file, self_offer, sha256, unpack_product)
from actor import Actor
from run import BOOT, FILES, exercise, exact_product

# Synthetic metadata only. No product has been qualified for this port.
SOURCE = "e1c30adbcf15af67675bd476c24efb713430a68b"
RUN = 17
ARTIFACT = 123


def app_schema(db):
    """Use the accepted source's real SQLite definitions for local witnesses."""
    root = Path(__file__).resolve().parents[2]
    schema = (root / "core/src/core/storage/schema.rs").read_text()
    db.executescript(schema.split('const INITIAL_SCHEMA: &str = r#"',1)[1].split('"#;',1)[0])
    checkpoint = (root / "chat-protocol/src/storage/checkpoint_sqlite.rs").read_text()
    db.executescript(re.search(r"CREATE TABLE ndr_kv_checkpoint_chunks\(.*?\);",checkpoint,re.S)[0])


class ProductTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.archive = self.root / "product.zip"
        self.run = {"id": RUN, "head_sha": SOURCE, "repository": {"full_name": REPO}, "status": "completed",
                    "conclusion": "success", "path": ".github/workflows/ci.yml"}
        self.tree = "c" * 40

    def spec(self, run=None, artifact=None, tree=None):
        return product_spec(run or self.run, artifact or self.artifact, tree or self.tree,
                            source=SOURCE, run_id=RUN, artifact_id=ARTIFACT)

    def pack(self, *, source=SOURCE, tree=None, member=None, digest=None, receipt_fields=None):
        tar = io.BytesIO()
        with tarfile.open(fileobj=tar, mode="w:gz") as t:
            for name, data in [(str(APP / "Contents/MacOS/Iris Chat"), b"app"), (str(CLI), b"cli")]:
                entry = tarfile.TarInfo(name)
                entry.size = len(data)
                entry.mode = 0o755
                t.addfile(entry, io.BytesIO(data))
            if member:
                t.addfile(member)
        body = tar.getvalue()
        receipt = {"commit": source, "tree": tree or self.tree, "architecture": "arm64", "platform": "macos",
                   "configuration": "Debug", "releasePackage": False, "archive": "macos-products.tar.gz",
                   "rustProfile": "debug", "signed": False, "testsExecuted": False,
                   "archiveBytes": len(body), "archiveSHA256": digest or hashlib.sha256(body).hexdigest()}
        receipt.update(receipt_fields or {})
        with zipfile.ZipFile(self.archive, "w") as bundle:
            bundle.writestr("build-receipt.json", json.dumps(receipt))
            bundle.writestr("macos-products.tar.gz", body)
        self.artifact = {"id": ARTIFACT, "name": "macos-validation-" + SOURCE, "expired": False,
                         "size_in_bytes": self.archive.stat().st_size, "digest": "sha256:" + sha256(self.archive),
                         "workflow_run": {"id": RUN, "head_sha": SOURCE}}
        return self.spec()

    def test_extract_only_verified_app_and_cli(self):
        out = unpack_product(self.archive, self.root, self.pack(), 0)
        self.assertEqual((out / APP / "Contents/MacOS/Iris Chat").read_bytes(), b"app")
        self.assertEqual((out / CLI).read_bytes(), b"cli")

    def test_reject_wrong_run_or_expired_artifact(self):
        self.pack()
        for key, value in [("status", "in_progress"), ("conclusion", "failure"), ("head_sha", "b"*40)]:
            with self.assertRaises(RuntimeError):
                self.spec(run=dict(self.run, **{key: value}))
        with self.assertRaises(RuntimeError):
            self.spec(artifact=dict(self.artifact, expired=True))

    def test_explicit_product_binding_rejects_substitutions(self):
        self.pack()
        for bad in [dict(self.artifact, id=124), dict(self.artifact, workflow_run={"id":18,"head_sha":SOURCE}),
                    dict(self.artifact, workflow_run={"id":RUN,"head_sha":"d"*40})]:
            with self.subTest(artifact=bad), self.assertRaises(RuntimeError):
                self.spec(artifact=bad)
        for source, run_id, artifact_id in [("main",RUN,ARTIFACT), (SOURCE,0,ARTIFACT), (SOURCE,RUN,0)]:
            with self.assertRaises(RuntimeError):
                product_spec(self.run,self.artifact,self.tree,source=source,run_id=run_id,artifact_id=artifact_id)

    def test_current_producer_workflows_and_repository_are_bound(self):
        self.pack()
        current = dict(self.run,path=".github/workflows/apple-validation-products.yml")
        self.assertEqual(self.spec(run=current)["source"],SOURCE)
        for bad in [dict(current,path=".github/workflows/release.yml"),
                    dict(current,repository={"full_name":"other/repository"})]:
            with self.assertRaises(RuntimeError):
                self.spec(run=bad)

    def test_metadata_uses_exact_artifact_id_and_commit(self):
        from unittest.mock import patch
        expected = self.pack()
        def api(route):
            return {f"actions/runs/{RUN}":self.run,f"actions/artifacts/{ARTIFACT}":self.artifact,
                    "git/commits/"+SOURCE:{"sha":SOURCE,"tree":{"sha":self.tree}}}[route]
        with patch("run.api",side_effect=api) as query:
            self.assertEqual(exact_product(SOURCE,RUN,ARTIFACT),expected)
            self.assertEqual(query.call_count,3)
        with patch("run.api") as query:
            with self.assertRaises(RuntimeError):
                exact_product("main",RUN,ARTIFACT)
            query.assert_not_called()

    def test_no_implicit_historical_product(self):
        proc = subprocess.run([sys.executable,Path(__file__).with_name("run.py")],
                              capture_output=True,text=True,timeout=5)
        self.assertEqual(proc.returncode,2)
        self.assertIn("--product-source",proc.stderr)
        self.assertIn("--product-run",proc.stderr)
        self.assertIn("--product-artifact",proc.stderr)

    def test_reject_wrong_receipt_source_and_tree(self):
        for overrides in [{"source":"wrong"}, {"tree":"b"*40}]:
            with self.assertRaisesRegex(RuntimeError, "receipt"):
                unpack_product(self.archive, self.root, self.pack(**overrides), 0)

    def test_reject_another_product_kind(self):
        for field,value in [("signed",True),("testsExecuted",True),("rustProfile","release"),
                            ("configuration","Release"),("architecture","x86_64"),("releasePackage",True)]:
            with self.subTest(field=field), self.assertRaisesRegex(RuntimeError,"receipt"):
                unpack_product(self.archive,self.root,self.pack(receipt_fields={field:value}),0)

    def test_reject_wrong_inner_and_outer_digests(self):
        spec = self.pack(digest="b"*64)
        with self.assertRaisesRegex(RuntimeError, "Outer"):
            unpack_product(self.archive, self.root, dict(spec, sha256="0"*64), 0)
        with self.assertRaisesRegex(RuntimeError, "Inner"):
            unpack_product(self.archive, self.root, spec, 0)

    def test_reject_escape(self):
        escape = tarfile.TarInfo(str(APP / "outside"))
        escape.type = tarfile.SYMTYPE
        escape.linkname = "/etc"
        spec = self.pack(member=escape)
        with self.assertRaises(tarfile.FilterError):
            unpack_product(self.archive, self.root, spec, 0)

    def test_respect_disk_floor(self):
        with self.assertRaisesRegex(RuntimeError, "available space"):
            unpack_product(self.archive, self.root, self.pack(), 1 << 60)


class OfferTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.owner, self.source = "a"*64, "b"*64
        self.ready, self.late = "c"*64, "d"*64
        self.name, self.body = "unique-note.bin", b"selected bytes"
        self.digest = hashlib.sha256(self.body).hexdigest()
        self.transfer = "e"*32
        self.db = self.root / "core.sqlite3"
        with closing(sqlite3.connect(self.db)) as db, db:
            app_schema(db)
            db.execute("PRAGMA foreign_keys=ON")
            db.execute("INSERT INTO threads(chat_id) VALUES(?)",(self.owner,))

    def insert(self, mid="message", *, signer=None, owner=None):
        manifest = {"files":[{"filename":self.name,"size_bytes":len(self.body),"sha256":self.digest}],
                    "id":self.transfer, "owner":owner or self.owner, "recipient":self.owner,
                    "device":self.source, "caption":"", "token":"fake"}
        wire = {"pubkey":signer or self.source,"created_at":1,"kind":21111,"tags":[],
                "content":json.dumps(manifest)}
        data = [wire["pubkey"], wire["created_at"], wire["kind"], wire["tags"], wire["content"]]
        wire["id"] = hashlib.sha256(json.dumps([0]+data,separators=(",",":")).encode()).hexdigest()
        with closing(sqlite3.connect(self.db)) as db, db:
            db.execute("INSERT INTO messages(id,chat_id,author_owner_pubkey_hex,is_outgoing,body,"
                       "kind,author,created_at_secs,delivery) VALUES(?,?,?,?,?,'user','Notes test',1,'sent')",
                       (mid,self.owner,self.owner,1,PREFIX+json.dumps(wire)))

    def read(self):
        return self_offer(self.root,self.owner,self.source,self.name)

    def test_offer_and_accepted_destination(self):
        self.insert()
        offer = self.read()
        compare_copies(offer,[offer.copy()],{"size":len(self.body),"sha256":self.digest})
        chosen = self.root/"picked"
        file = chosen/("Iris files "+self.transfer)/("1-"+self.name)
        file.parent.mkdir(parents=True)
        file.write_bytes(self.body)
        self.assertEqual(received_file(chosen,offer,self.name),self.digest)
        file.write_bytes(bytes(len(self.body)))
        with self.assertRaisesRegex(RuntimeError,"bytes"):
            received_file(chosen,offer,self.name)
        file.write_bytes(self.body)
        (chosen/"extra").touch()
        with self.assertRaisesRegex(RuntimeError,"Unexpected"):
            received_file(chosen,offer,self.name)

    def test_duplicate_and_missing_offer(self):
        self.assertIsNone(self.read())
        self.insert("first")
        self.insert("duplicate")
        with self.assertRaisesRegex(RuntimeError,"Duplicate"):
            self.read()

    def test_wrong_sender_and_content(self):
        self.insert(signer=self.late)
        with self.assertRaisesRegex(RuntimeError,"source device"):
            self.read()
        with closing(sqlite3.connect(self.db)) as db, db:
            db.execute("DELETE FROM messages")
        self.insert()
        with closing(sqlite3.connect(self.db)) as db, db:
            db.execute("UPDATE messages SET body=replace(body,'unique-note.bin','renamed.bin')")
        with self.assertRaisesRegex(RuntimeError,"content ID"):
            self_offer(self.root,self.owner,self.source,"renamed.bin")

    def test_reject_different_copies(self):
        self.insert()
        offer = self.read()
        for field in ["transfer","message","wire","size","sha256"]:
            with self.assertRaises(RuntimeError):
                compare_copies(offer,[dict(offer,**{field: "changed"})],offer)

    def test_pending_is_for_exact_delivery_and_unready_device(self):
        pending = {"chat_id":self.owner,"message_id":"message",
                   "eligible_devices":[self.ready,self.late],"completed_devices":[self.ready]}
        for finished,expected in [([self.ready],True),([],False),([self.late],False),
                                  ([self.ready,self.late],False)]:
            pending["completed_devices"] = finished
            tail = b'],' + json.dumps({"pending_local_sibling_sends":[pending]}).encode()[1:]
            with closing(sqlite3.connect(self.db)) as db, db:
                db.execute("DELETE FROM ndr_kv_checkpoint_chunks")
                for i,chunk in enumerate([tail[:25],tail[25:]]):
                    db.execute("INSERT INTO ndr_kv_checkpoint_chunks VALUES(?,?,?,?,?,?)",
                               (self.owner,self.source,"appcore/protocol-engine-state-v1",b"\0core",i,chunk))
            self.assertEqual(sibling_retry_state(self.root,self.owner,self.source,self.ready,self.late,"message"),
                             "receiver-complete" if expected else "other")
            self.assertEqual(sibling_retry_state(self.root,self.owner,self.source,self.ready,self.late,"wrong"),"drained")


class FixturePeer:
    """Only for verifying coordination, ordering and fail-closed assertions."""
    def __init__(self, role, steps):
        self.role, self.steps = role,steps
        self.overrides = {}

    def call(self, action, *args):
        self.steps.append((self.role,action))
        if action in self.overrides:
            return copy.deepcopy(self.overrides[action])
        accounts = {"sender":"S","receiver":"R","late":"L"}
        value = {"owner":"O","device":accounts[self.role]}
        offer = {"message":"M","wire":"W","transfer":"T","size":19,"sha256":"file"}
        if action=="approve":
            return ["S","R","L"] if args[0]=="late" else ["S","R"]
        if action=="request":
            return args[0]
        if action=="check_live":
            return "file" if args[1] else True
        return {"preflight":self.role,"bind":{"app":"app","cli":"cli"},"send":{"size":19,"sha256":"file"},
                "offer":offer,"accept":{"sha256":"file","images":{}},"create":value,"authorized":value,"resume":{"owner":"O","device":"L"},
                "launch":None,"retry":None}[action]


class WorkerTests(unittest.TestCase):
    def test_first_rpc_can_follow_source_on_same_pipe(self):
        root = Path(__file__).parent
        code = {name:(root / name).read_text() for name in FILES}
        wire = json.dumps(code) + "\n" + json.dumps(["not-an-operation", []]) + "\n"
        proc = subprocess.run([sys.executable, "-u", "-c", BOOT], input=wire, text=True,
                              capture_output=True, timeout=10, check=True)
        self.assertEqual(json.loads(proc.stdout), {"ok":False,"error":"Unknown actor operation"})

    def test_only_task_owned_late_worker_pauses_and_resumes(self):
        # A disposable fake service tests OS process handling; no Iris account
        # or device linking is used by this control.
        with tempfile.TemporaryDirectory() as tmp:
            actor = Actor(Path(tmp))
            actor.cli = Path(tmp)/"fixture-cli"
            actor.cli.write_text("#!" + sys.executable + "\n" + """
import json, sys, time
if sys.argv[-2:] == ["service","run"]:
    print('{"data":{"ready":true}}',flush=True)
    time.sleep(60)
elif sys.argv[-2:] == ["link","create"]:
    print(json.dumps({"status":"ok","data":{"url":"disposable-test"}}))
elif sys.argv[-1]=="state":
    print(json.dumps({"status":"ok","data":{"account":{"user_id":"primary","device_id":"late","device_state":"authorized"}}}))
else:
    sys.exit(1)
""")
            actor.cli.chmod(0o700)
            try:
                self.assertEqual(actor.request("late"), "disposable-test")
                proc = actor.procs["late"]
                self.assertTrue(actor.paused)
                self.assertIn("T",subprocess.check_output(["ps","-p",str(proc.pid),"-o","state="],text=True))
                self.assertEqual(actor.resume(), {"owner":"primary","device":"late"})
                self.assertFalse(actor.paused)
            finally:
                actor.close()
            self.assertIsNotNone(proc.poll())

    def test_cli_state_conversion_is_profile_local(self):
        from unittest.mock import patch, Mock
        with tempfile.TemporaryDirectory() as root:
            actor = Actor(Path(root))
            actor.run = "unique"
            actor.owner = "primary"
            actor.app = Path(root)/"app"
            profile = Path(root)/"main"
            profile.mkdir()
            (profile/"cli-account.json").write_text(json.dumps({"owner_pubkey_hex":"primary","owner_nsec":None,
                                                                "device_nsec":"synthetic-only"}))
            with closing(sqlite3.connect(profile/"core.sqlite3")) as db, db:
                db.execute("CREATE TABLE app_keys(owner_pubkey_hex,devices_json)")
                db.execute("INSERT INTO app_keys VALUES(?,?)",("primary",json.dumps(
                    [{"identity_pubkey_hex":i} for i in ["a","b","c"]])))
            actor.procs["main"]=Mock()
            actor.procs["main"].wait.return_value=0
            with patch.object(actor,"cli_call"),patch.object(actor,"ui") as ui,patch("actor.subprocess.Popen") as launch:
                actor.launch(["a","b","c"])
                env=launch.call_args.kwargs["env"]
                self.assertEqual(env["IRIS_UI_TEST_DATA_DIR"],str(profile))
                self.assertNotIn("IRIS_UI_TEST_RESET",env)
                self.assertEqual(env["IRIS_DISABLE_NOTIFICATIONS"],"1")
                self.assertEqual(json.loads((profile/"account-secret.json").read_text()),
                                 {"ownerNsec":None,"ownerPubkeyHex":"primary","deviceNsec":"synthetic-only"})
                ui.assert_called_once_with("open-self")


class CoordinatorTests(unittest.TestCase):
    def setUp(self):
        self.steps = []
        self.sender = FixturePeer("sender",self.steps)
        self.receiver = FixturePeer("receiver",self.steps)
        self.opts = SimpleNamespace(sender_user="sandbox1",receiver_user="sandbox2",sender_archive="a",
                                    receiver_archive="b",reserve_gib=12,screenshots=False)

    def go(self):
        proof = {}
        exercise(self.sender,self.receiver,{},self.opts,proof,"run",settle=lambda _:None)
        return proof

    def test_order_and_proof(self):
        proof = self.go()
        self.assertTrue(proof["bothAppsStayedOpen"])
        self.assertEqual(proof["copiesPerDevice"],[1,1,1])
        self.assertLess(self.steps.index(("sender","retry")),self.steps.index(("receiver","accept")))
        self.assertLess(self.steps.index(("receiver","accept")),self.steps.index(("receiver","resume")))

    def test_same_instance_cannot_pass(self):
        self.sender.overrides["preflight"] = "receiver"
        with self.assertRaisesRegex(RuntimeError,"independent"):
            self.go()
        self.assertNotIn(("receiver","create"),self.steps)

    def test_different_product_cannot_pass(self):
        self.sender.overrides["bind"] = {"app":"different","cli":"cli"}
        with self.assertRaisesRegex(RuntimeError,"identical"):
            self.go()

    def test_wrong_download_fails_before_resume(self):
        self.receiver.overrides["accept"] = {"sha256":"wrong", "images":{}}
        with self.assertRaisesRegex(RuntimeError,"different bytes"):
            self.go()
        self.assertNotIn(("receiver","resume"),self.steps)

    def test_wrong_late_identity_fails(self):
        self.receiver.overrides["resume"] = {"owner":"O","device":"new"}
        with self.assertRaisesRegex(RuntimeError,"resumed device"):
            self.go()


if __name__ == "__main__":
    unittest.main()
