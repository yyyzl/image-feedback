"""Windows-only, offline handoff integration test. NEVER runs a real installer.

Build the debug app first: cargo build
Run: python scripts/test-update-handoff.py

Uses isolated LOCALAPPDATA, real editor/tray/helper processes, and a tiny Rust
fixture that only writes a marker. A debug-only test mutex isolates running apps.
Only processes started by this script may be terminated during cleanup.
"""
import ctypes
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import socket
import subprocess
import tempfile
import threading
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
EXE = ROOT / "target/debug/image-feedback-slint.exe"
REPORT = ROOT / "artifacts/update-handoff/report.json"


def wait_for(check, seconds=30):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if check():
            return
        time.sleep(0.08)
    raise AssertionError("timed out waiting for test condition")


def quit_peer(directory, endpoint):
    data = json.loads((directory / endpoint).read_text(encoding="utf-8"))
    with socket.create_connection(("127.0.0.1", data["port"]), timeout=5) as stream:
        stream.sendall(b"Q" + data["token"].encode("ascii"))
        assert stream.recv(1) == b"Q", "peer did not consent to exit"


def reply(process):
    results = queue.Queue()
    threading.Thread(target=lambda: results.put(process.stdout.readline()), daemon=True).start()
    return results.get(timeout=30).decode("utf-8").strip()


def main():
    assert os.name == "nt", "Windows only"
    assert EXE.is_file(), "run cargo build first"
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenMutexW.argtypes = [ctypes.c_uint32, ctypes.c_int, ctypes.c_wchar_p]
    kernel.OpenMutexW.restype = ctypes.c_void_p
    kernel.CloseHandle.argtypes = [ctypes.c_void_p]
    instance = uuid.uuid4().hex
    existing = kernel.OpenMutexW(0x00100000, False, f"Local\\ImageFeedback.Test.{instance}")
    if existing:
        kernel.CloseHandle(existing)
        raise SystemExit("An application is already running; it was preserved. Close it before this test.")
    assert ctypes.get_last_error() == 2, "could not safely check running processes"
    processes = []
    with tempfile.TemporaryDirectory(prefix="image-feedback-handoff-") as temporary:
        root = Path(temporary)
        directory = root / "data/image-feedback-slint"
        package = directory / "updates/fixture/ImageFeedback-99.0.0-windows-x64-setup.exe"
        package.parent.mkdir(parents=True)
        marker = root / "fixture-ran.txt"
        source = root / "fixture.rs"
        source.write_text('fn main() { let p = std::env::var_os("UPDATE_TEST_MARKER").unwrap(); std::fs::write(p, b"fixture only; no installation").unwrap(); }', encoding="utf-8")
        subprocess.run(["rustc", str(source), "-o", str(package)], check=True)
        digest = hashlib.sha256(package.read_bytes()).hexdigest()
        helper = root / "helper.exe"
        shutil.copyfile(EXE, helper)
        env = dict(os.environ, LOCALAPPDATA=str(root / "data"), UPDATE_TEST_MARKER=str(marker),
                   IMAGE_FEEDBACK_TEST_INSTANCE=instance)
        log_path = ROOT / "artifacts/update-handoff/processes.log"
        log_path.parent.mkdir(parents=True, exist_ok=True)
        with log_path.open("wb") as log:
            def spawn(args, pipes=False):
                p = subprocess.Popen(args, env=env, stdin=subprocess.PIPE if pipes else subprocess.DEVNULL,
                                     stdout=subprocess.PIPE if pipes else log, stderr=log)
                processes.append(p)
                return p
            try:
                tray = spawn([str(EXE), "--tray-only", "--update-offline-test"])
                wait_for(lambda: (directory / "resident-control.json").exists())
                editor = spawn([str(EXE), "--editor", "--update-offline-test", "--software-rendering"])
                wait_for(lambda: (directory / "editor-control.json").exists())
                plan = {"package": {"path": str(package), "sha256": digest, "size": package.stat().st_size},
                        "directory": str(directory), "original_exe": str(EXE), "editor_pid": editor.pid}

                def start_helper(payload):
                    p = spawn([str(helper), "--install-update-helper"], pipes=True)
                    p.stdin.write((json.dumps(payload) + "\n").encode("utf-8"))
                    p.stdin.flush()
                    return p

                # Tampering is rejected before either live process is asked to exit.
                bad = json.loads(json.dumps(plan))
                bad["package"]["sha256"] = "0" * 64
                failed = start_helper(bad)
                assert reply(failed).startswith("ERROR ")
                failed.stdin.close()
                failed.wait(timeout=5)
                assert editor.poll() is None and tray.poll() is None and not marker.exists()

                # A helper which prepared successfully but never received COMMIT does nothing.
                cancelled = start_helper(plan)
                assert reply(cancelled) == "READY"
                cancelled.stdin.close()
                cancelled.wait(timeout=5)
                assert editor.poll() is None and tray.poll() is None and not marker.exists()

                # COMMIT alone cannot execute the package while the editor is alive.
                committed = start_helper(plan)
                assert reply(committed) == "READY"
                committed.stdin.write(b"INSTALL\n")
                committed.stdin.flush()
                committed.stdin.close()
                time.sleep(0.5)
                assert editor.poll() is None and tray.poll() is None and not marker.exists()
                quit_peer(directory, "editor-control.json")
                editor.wait(timeout=10)
                wait_for(marker.exists)
                assert tray.poll() is not None, "fixture launched before tray process exited"
                committed.wait(timeout=10)
                assert committed.returncode == 0
                assert marker.read_text() == "fixture only; no installation"
                REPORT.write_text(json.dumps({"result": "passed", "network": False, "real_installer_executed": False,
                    "checks": ["real editor and resident IPC", "checksum failure preserves both processes",
                               "uncommitted helper does not install", "commit waits for editor process exit",
                               "resident exits without a force kill", "fixture executes only after both processes exit"],
                    "fixture": "locally compiled Rust program that only writes a marker"}, indent=2), encoding="utf-8")
                print(f"PASSED: {REPORT}")
            finally:
                for p in reversed(processes):
                    if p.poll() is None:
                        p.terminate()  # only this test's retained Popen children
                        p.wait(timeout=10)
                    if p.stdin and not p.stdin.closed:
                        p.stdin.close()
                    if p.stdout:
                        p.stdout.close()


if __name__ == "__main__":
    main()
