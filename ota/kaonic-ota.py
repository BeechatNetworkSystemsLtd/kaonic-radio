from flask import Flask, request, jsonify
from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import rsa, ed25519, padding
from pathlib import Path

import hashlib
import logging
import os
import shutil
import stat
import subprocess
import tempfile
import time
import zipfile

BINARY_PATH = Path("/usr/bin")
APP_PATH = BINARY_PATH / "kaonic-commd"

METADATA_PATH = Path("/etc/kaonic")
VERSION_PATH = METADATA_PATH / "kaonic-commd.version"
HASH_PATH = METADATA_PATH / "kaonic-commd.sha256"
BACKUP_PATH = METADATA_PATH / "kaonic-commd.bak"
VERIFY_KEY_PATH = METADATA_PATH / "beechat-ota.pub.pem"

logger = logging.getLogger("kaonic-ota")
logging.basicConfig(level=logging.INFO)

app = Flask(__name__)

def validate_app_file():
    logger.info("Validate app file")
    if not METADATA_PATH.exists():
        logger.info("Metadata path is not existing. Creating empty one")
        METADATA_PATH.mkdir(parents=True)
        return

    if not APP_PATH.exists() and not BACKUP_PATH.exists():
        logger.info("No kaonic_commd and kaonic_commd.bak")
        return

    expected_hash = HASH_PATH.read_text().strip() if HASH_PATH.exists() else ""
    actual_hash = sha256sum(APP_PATH) if APP_PATH.exists() else ""

    if (not APP_PATH.exists() or not HASH_PATH.exists()) or (expected_hash != actual_hash):
        logger.warning("Restoring app from the backup")
        stop_kaonic_commd()
        restore_backup()
        launch_kaonic_commd()

def validate_signature(file_path:Path, sig_path:Path, key_path:Path):
    try:
        public_key = serialization.load_pem_public_key(key_path.read_bytes())
        file_data = file_path.read_bytes()
        signature = sig_path.read_bytes()

        if isinstance(public_key, rsa.RSAPublicKey):
            public_key.verify(
                signature,
                file_data,
                padding.PKCS1v15(),
                hashes.SHA256()
            )
        elif isinstance(public_key, ed25519.Ed25519PublicKey):
            public_key.verify(signature, file_data)
        else:
            return False

        logger.info("Signature has been verified")
        return True
    except InvalidSignature:
        logger.error("Signature verification failed")
        return False
    except Exception as e:
        logger.error(f"Error during signature verification: {e}")
        return False

@app.route("/api/ota/commd/upload", methods=["POST"])
def upload_ota():
    if 'file' not in request.files:
        return jsonify({"detail": "No file uploaded"}), 400
    file = request.files['file']
    if file.content_type != "application/x-zip-compressed":
        return jsonify({"detail": "Only ZIP files accepted"}), 400

    if not VERIFY_KEY_PATH.exists():
        return jsonify({"detail": "OTA certificate is not present"}), 500

    with tempfile.TemporaryDirectory() as tmp_dir_str:
        temp_dir = Path(tmp_dir_str)
        zip_path = temp_dir / "upload.zip"
        app_path = temp_dir / "kaonic-commd"
        hash_path = temp_dir / "kaonic-commd.sha256"
        sig_path = temp_dir / "kaonic-commd.sig"
        version_path = temp_dir / "kaonic-commd.version"

        file.save(zip_path)

        try:
            logger.info("Uploaded zip extraction")
            with zipfile.ZipFile(zip_path, "r") as zip_ref:
                zip_ref.extractall(temp_dir)
                # List extracted files for debugging using os.listdir
                logger.info(f"Extracted files: {os.listdir(temp_dir)}")
        except zipfile.BadZipFile:
            logger.error("Zip wasn't extracted")
            return jsonify({"detail": "Invalid ZIP file"}), 400

        logger.info("Zip content validation")
        required_files = ["kaonic-commd", "kaonic-commd.sha256", "kaonic-commd.version", "kaonic-commd.sig"]
        for req_file in required_files:
            if not (temp_dir / req_file).exists():
                logger.error(f"Missing {req_file} in OTA package")
                return jsonify({"detail": f"Missing {req_file} in OTA package"}), 400

        expected_hash = hash_path.read_text().strip()
        actual_hash = sha256sum(app_path)

        logger.info(f"Expected hash: {expected_hash}")
        logger.info(f"Actual hash: {actual_hash}")

        if expected_hash != actual_hash:
            logger.error("SHA256 hash mismatch")
            return jsonify({"detail": "SHA256 hash mismatch"}), 400

        if not validate_signature(app_path, sig_path, VERIFY_KEY_PATH):
            logger.error("Signature wasn't validated")
            return jsonify({"detail": "SHA256 hash mismatch"}), 400

        stop_kaonic_commd()
        backup_current()

        shutil.copy2(app_path, APP_PATH)

        logger.info("Changing app file permissions")
        current_permissions = APP_PATH.stat().st_mode
        os.chmod(APP_PATH, current_permissions | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

        launch_kaonic_commd()

        logger.info("Checking kaonic-commd service status via systemctl")
        for _ in range(10):
            if is_kaonic_running():
                time.sleep(1)
            else:
                logger.error("Application validation failed")
                break

        if is_kaonic_running():
            shutil.copy2(hash_path, HASH_PATH)
            shutil.copy2(version_path, VERSION_PATH)
            logger.info("Updated successfully")
            return jsonify({"detail": "Update successful"})
        else:
            restore_backup()
            launch_kaonic_commd()
            logger.error("Failed to start new app, rollback done")
            return jsonify({"detail": "Failed to start new app, rollback done"}), 500

@app.route("/api/ota/commd/version", methods=["GET"])
def get_version():
    if not VERSION_PATH.exists() or not HASH_PATH.exists():
        return jsonify({"version": None, "hash": None})

    version = VERSION_PATH.read_text().strip()
    hash_val = HASH_PATH.read_text().strip()
    return jsonify({"version": version, "hash": hash_val})

def sha256sum(file_path: Path) -> str:
    h = hashlib.sha256()
    with file_path.open("rb") as f:
        for chunk in iter(lambda: f.read(4096), b""):
            h.update(chunk)
    return h.hexdigest()

def stop_kaonic_commd():
    logger.info("Stopping kaonic-commd service via systemctl")
    subprocess.run(
        ["systemctl", "stop", "kaonic-commd.service"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False
    )

def is_kaonic_running() -> bool:
    result = subprocess.run(
        ["systemctl", "is-active", "--quiet", "kaonic-commd.service"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL
    )
    return result.returncode == 0

def launch_kaonic_commd():
    logger.info("Starting kaonic-commd service via systemctl")
    subprocess.run(
        ["systemctl", "start", "kaonic-commd.service"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False
    )

def backup_current():
    logger.info("Backup current executable")
    if BACKUP_PATH.exists():
        BACKUP_PATH.unlink()
    if APP_PATH.exists():
        shutil.copy2(APP_PATH, BACKUP_PATH)

def restore_backup():
    logger.info("Restoring backup")
    if BACKUP_PATH.exists():
        shutil.copy2(BACKUP_PATH, APP_PATH)
        logger.info("Backup restored")
    else:
        logger.info("Backup file is absent")

if __name__ == "__main__":
    validate_app_file()
    app.run(host="0.0.0.0", port=8682)

